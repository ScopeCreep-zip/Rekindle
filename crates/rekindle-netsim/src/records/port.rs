//! `RecordPort` for [`FakePool`]: the calls a session makes, over the
//! helpers of the parent module.

use rekindle_records::lease::{max_subkey_bytes, LeaseId, ReleasePlan, SchemaShape, WatchPlan};
use rekindle_records::port::{
    PortError, RecordPort, RecordReport, RecordSchema, RecordValue, ReportScope, SetOutcome,
    Settled, UnsentWrite, ValueChange, ValueChangeReport,
};
use rekindle_utils::RandomSource;

use super::{new_key_text, subkey_count, writer_public, FakePool, Held, Phase};
use crate::latency;
use crate::network::NetRecord;

#[async_trait::async_trait]
impl RecordPort for FakePool {
    async fn acquire(&self, key: &str, writer: Option<&str>) -> Result<LeaseId, PortError> {
        self.check_open(true)?;
        if let Some(writer) = writer {
            writer_public(writer)?;
        }
        {
            let inner = self.net.inner.lock();
            let visible = inner
                .records
                .get(key)
                .is_some_and(|r| r.published || r.creator == self.node);
            if !visible {
                return Err(PortError::Unreachable(key.to_string()));
            }
        }
        // Veilid's open inspects the record on the network (`open_record.rs:60`).
        let _slot = self.latency(self.read_span(key, None), key, &[]).await;
        self.reach(key)?;
        let (lease, _) = self
            .state
            .lock()
            .leases
            .acquire(key, writer.map(ToString::to_string));
        Ok(lease)
    }

    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&str>,
    ) -> Result<(LeaseId, String, String), PortError> {
        self.check_open(true)?;
        let _slot = self.latency(latency::CREATE, "", &[]).await;
        let count = schema.subkey_count();
        if count == 0 || count > 1024 {
            return Err(PortError::Dht(format!("schema: {count} subkeys")));
        }
        let owner = owner.map_or_else(
            || {
                let mut secret = [0u8; 32];
                self.rng.fill(&mut secret);
                rekindle_crypto::keypair_text::keypair_text_from_signing(
                    &rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&secret),
                )
            },
            ToString::to_string,
        );
        let public = writer_public(&owner)?;
        let key = new_key_text(self.rng.as_ref());
        {
            let mut inner = self.net.inner.lock();
            inner.records.insert(
                key.clone(),
                NetRecord {
                    schema: schema.clone(),
                    owner: public,
                    creator: self.node,
                    published: false,
                    values: vec![None; count],
                },
            );
            inner.owners.insert(key.clone(), owner.clone());
        }
        let (lease, _) = self.state.lock().leases.acquire(&key, Some(owner.clone()));
        Ok((lease, key, owner))
    }

    fn key_of(&self, lease: LeaseId) -> Option<String> {
        self.state.lock().leases.key(lease).map(ToString::to_string)
    }

    async fn release(&self, lease: LeaseId) {
        let plan = self.state.lock().leases.release_plan(lease);
        match plan {
            ReleasePlan::Kept => {}
            ReleasePlan::Closed(key) => self.apply_watch(&key, WatchPlan::Cancel),
            ReleasePlan::Rewatch(key, plan) => self.apply_watch(&key, plan),
        }
    }

    async fn get(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, PortError> {
        self.check_open(true)?;
        let key = self.key(lease)?;
        let local = self.local(&key, subkey);
        if !force_refresh && local.is_some() {
            return Ok(local);
        }
        let span = self.read_span(&key, Some(&[subkey]));
        let _slot = self.latency(span, &key, &[subkey]).await;
        self.reach(&key)?;
        if self
            .net
            .inner
            .lock()
            .faults
            .stale_reads
            .contains(&self.node)
        {
            return Ok(local);
        }
        Ok(self.refresh(&key, subkey))
    }

    async fn set(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        self.check_open(true)?;
        let key = self.key(lease)?;
        let _slot = self.latency(latency::SET, &key, &[subkey]).await;
        self.write(&key, subkey, data, writer)
    }

    async fn set_durable(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        self.check_open(true)?;
        let key = self.key(lease)?;
        let _slot = self.latency(latency::SET, &key, &[subkey]).await;
        let outcome = self.write(&key, subkey, data.clone(), writer)?;
        if matches!(outcome, SetOutcome::Offline | SetOutcome::BelowConsensus) {
            let mut state = self.state.lock();
            state.held.retain(|h| !(h.key == key && h.subkey == subkey));
            state.held.push(Held {
                key,
                subkey,
                data,
                writer: writer.map(ToString::to_string),
            });
        }
        Ok(outcome)
    }

    async fn inspect(
        &self,
        lease: LeaseId,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, PortError> {
        self.check_open(true)?;
        let key = self.key(lease)?;
        let _slot = if scope == ReportScope::Local {
            None
        } else {
            let slot = self.latency(self.read_span(&key, subkeys), &key, &[]).await;
            self.reach(&key)?;
            slot
        };
        let inner = self.net.inner.lock();
        let record = inner
            .records
            .get(&key)
            .ok_or_else(|| PortError::Unreachable(key.clone()))?;
        let subkeys: Vec<u32> = match subkeys {
            Some(s) => s.to_vec(),
            None => (0..subkey_count(&record.schema)).collect(),
        };
        let local = &inner.nodes[self.node].local;
        let local_seqs = subkeys
            .iter()
            .map(|s| local.get(&(key.clone(), *s)).map(|v| v.seq))
            .collect();
        let stale = inner.faults.stale_reads.contains(&self.node);
        let network_seqs = subkeys
            .iter()
            .map(|s| {
                if scope == ReportScope::Local || stale {
                    None
                } else {
                    record
                        .values
                        .get(*s as usize)
                        .cloned()
                        .flatten()
                        .map(|v| v.seq)
                }
            })
            .collect();
        drop(inner);
        let offline_subkeys = self
            .state
            .lock()
            .held
            .iter()
            .filter(|h| h.key == key)
            .map(|h| h.subkey)
            .collect();
        Ok(RecordReport {
            subkeys,
            local_seqs,
            network_seqs,
            offline_subkeys,
        })
    }

    async fn read_changed(
        &self,
        lease: LeaseId,
        subkeys: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError> {
        self.check_open(true)?;
        let key = self.key(lease)?;
        // An inspect, then the gets of what changed.
        let span = self.read_span(&key, Some(subkeys));
        drop(self.latency(span, &key, &[]).await);
        let _slot = if span == latency::GET {
            self.latency(latency::GET, &key, subkeys).await
        } else {
            None
        };
        self.reach(&key)?;
        let stale = self
            .net
            .inner
            .lock()
            .faults
            .stale_reads
            .contains(&self.node);
        Ok(subkeys
            .iter()
            .filter_map(|s| {
                let value = if stale {
                    self.local(&key, *s)
                } else {
                    self.refresh(&key, *s)
                };
                value.map(|v| (*s, v))
            })
            .collect())
    }

    async fn watch(&self, lease: LeaseId, subkeys: &[u32]) -> Result<(), PortError> {
        self.check_open(true)?;
        let key = self.key(lease)?;
        let _slot = self.latency(latency::WATCH, &key, &[]).await;
        let plan = self
            .state
            .lock()
            .leases
            .set_watch(lease, subkeys.iter().copied().collect());
        self.apply_watch(&key, plan);
        Ok(())
    }

    async fn watch_all(&self, lease: LeaseId) -> Result<(), PortError> {
        let key = self.key(lease)?;
        let count = self
            .net
            .inner
            .lock()
            .records
            .get(&key)
            .map(|r| subkey_count(&r.schema))
            .ok_or_else(|| PortError::Unreachable(key.clone()))?;
        let all: Vec<u32> = (0..count).collect();
        self.watch(lease, &all).await
    }

    fn subkey_cap(&self, lease: LeaseId) -> Result<usize, PortError> {
        let key = self.key(lease)?;
        let inner = self.net.inner.lock();
        let record = inner
            .records
            .get(&key)
            .ok_or_else(|| PortError::Unreachable(key.clone()))?;
        Ok(max_subkey_bytes(SchemaShape {
            subkey_count: subkey_count(&record.schema),
        }))
    }

    async fn rehydrate(&self, lease: LeaseId) -> Result<(), PortError> {
        self.check_open(true)?;
        let key = self.key(lease)?;
        self.reach(&key)?;
        let present = self.inspect_present(lease).await?;
        let _slot = self.latency(latency::GET, &key, &present).await;
        for subkey in present {
            self.refresh(&key, subkey);
        }
        Ok(())
    }

    fn on_value_change(&self, change: &ValueChange) -> ValueChangeReport {
        let (held, watched) = {
            let state = self.state.lock();
            (
                state.leases.is_held(&change.key),
                state.leases.watched(&change.key),
            )
        };
        if let (Some(value), Some(first)) = (&change.value, change.subkeys.first()) {
            let mut inner = self.net.inner.lock();
            if let Some(slot) = inner.nodes.get_mut(self.node) {
                let entry = slot.local.entry((change.key.clone(), *first));
                match entry {
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        if value.seq > e.get().seq {
                            e.insert(value.clone());
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(value.clone());
                    }
                }
            }
        }
        let watch_died = change.count == 0 || change.subkeys.is_empty();
        if held && watch_died && !watched.is_empty() {
            // The pool re-arms a dead watch while it holds the record.
            self.apply_watch(&change.key, WatchPlan::Watch(watched));
        }
        ValueChangeReport {
            held,
            watch_died,
            subkeys: change.subkeys.clone(),
            value: change.value.clone(),
        }
    }

    fn settled(&self) -> tokio::sync::broadcast::Receiver<Settled> {
        self.settled.subscribe()
    }

    fn held_writes(&self) -> Vec<(String, u32)> {
        self.state
            .lock()
            .held
            .iter()
            .map(|h| (h.key.clone(), h.subkey))
            .collect()
    }

    fn take_held(&self) -> Vec<UnsentWrite> {
        std::mem::take(&mut self.state.lock().held)
            .into_iter()
            .map(|h| UnsentWrite {
                record_key: h.key,
                subkey: h.subkey,
                data: h.data,
            })
            .collect()
    }

    fn restore_held(&self, writes: Vec<UnsentWrite>) {
        self.state
            .lock()
            .held
            .extend(writes.into_iter().map(|w| Held {
                key: w.record_key,
                subkey: w.subkey,
                data: w.data,
                writer: None,
            }));
    }

    fn drain(&self) {
        let mut state = self.state.lock();
        if state.phase != Phase::Ended {
            state.phase = Phase::Draining;
        }
    }

    fn admit_teardown(&self) {
        let mut state = self.state.lock();
        if state.phase == Phase::Draining {
            state.phase = Phase::Teardown;
        }
    }

    fn end(&self) -> usize {
        let keys = {
            let mut state = self.state.lock();
            state.phase = Phase::Ended;
            state.leases.clear()
        };
        let mut inner = self.net.inner.lock();
        if let Some(slot) = inner.nodes.get_mut(self.node) {
            for key in &keys {
                slot.watches.remove(key);
            }
        }
        keys.len()
    }
}
