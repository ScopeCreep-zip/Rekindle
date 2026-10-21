//! Real Hosts on the network simulator (plan C9.3): each node is a
//! `Host::start` with `TransportSource::Ports` on a temp data root, with the
//! simulator's clock and generator, so the Host's adapters and engines
//! (governance runtime, gossip, MEK rotation) run as in production.
//! Requests go through `HostApi::call`; a held request is proved as the
//! CLI's `helpers::send` proves it.

use std::sync::Arc;
use std::time::Duration;

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, ConnId, HostApi};
use rekindle_netsim::Netsim;
use rekindle_node::host::{claim, Host, HostConfig, HostStores, TransportSource};
use rekindle_types::intent::{IntentProof, IntentReview, ProofRequired};
use rekindle_types::subscription_events::{FriendEvent, StreamFrame, SubscriptionEvent};

/// Every simulated identity's passphrase.
pub const PASSPHRASE: &str = "netsim passphrase";

/// A simulated node running a real Host.
pub struct SimNode {
    pub index: usize,
    pub host: Arc<Host>,
    /// The unlocked identity's public key, once [`unlock_new_identity`] ran.
    pub public_key: Option<String>,
    _dir: tempfile::TempDir,
}

/// A request the host refused.
#[derive(Debug)]
pub struct Refused {
    pub code: u32,
    pub message: String,
}

fn temp_root(dir: &tempfile::TempDir) -> rekindle_db::paths::DataRoot {
    let data = dir.path().join("data");
    rekindle_db::paths::DataRoot {
        config: dir.path().join("config"),
        logs: dir.path().join("logs"),
        state: data.join("state"),
        data,
    }
}

/// The caller every scenario request comes from: a bus client with admin
/// clearance, as the CLI on the same machine.
fn caller() -> Caller {
    Caller {
        conn: ConnId::Bus(1),
        clearance: SecurityLevel::Admin,
        verified_name: None,
        static_key: None,
    }
}

/// `n` nodes on one network for run `seed`, each running a Host; the pump
/// is started (every node attached).
pub async fn start(n: usize, seed: u64) -> (Netsim, Vec<SimNode>) {
    // `RUST_LOG` shows a scenario's Host logs (`--nocapture`).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let (sim, nodes) = Netsim::nodes(n, seed);
    let mut hosts = Vec::with_capacity(n);
    for node in nodes {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = temp_root(&dir);
        let storage_dir = root.data.join("veilid").display().to_string();
        let host = Host::start(HostConfig {
            root: claim(root).expect("claim the temp root"),
            stores: HostStores::default(),
            transport: TransportSource::Ports {
                config: rekindle_types::config::TransportConfig {
                    storage_dir,
                    ..Default::default()
                },
                ports: node.ports,
                events: node.events,
            },
            serve_bus: false,
            clock: Arc::clone(&node.clock) as Arc<dyn rekindle_utils::Clock>,
            rng: Arc::clone(&node.rng) as Arc<dyn rekindle_utils::RandomSource>,
            shell: None,
        })
        .await
        .expect("host starts");
        hosts.push(SimNode {
            index: node.index,
            host,
            public_key: None,
            _dir: dir,
        });
    }
    let _pump = sim.start();
    sim.quiesce().await;
    (sim, hosts)
}

/// The proof `review` asks for, as the CLI gives it with `--yes`,
/// `--confirm=<subject>` and the identity's passphrase.
fn prove(review: &IntentReview) -> IntentProof {
    match review.proof {
        ProofRequired::None | ProofRequired::Commit => IntentProof::Commit,
        ProofRequired::Acknowledge => IntentProof::Acknowledged,
        ProofRequired::TypeSubject => IntentProof::Typed(review.subject.typed_name().to_owned()),
        ProofRequired::Passphrase => IntentProof::Passphrase(PASSPHRASE.to_string().into()),
    }
}

impl SimNode {
    /// Send `request`; prove and commit it whenever the gate holds it.
    pub async fn call(&self, request: IpcRequest) -> Result<serde_json::Value, Refused> {
        let mut answer = self.host.call(caller(), request).await;
        loop {
            match answer {
                IpcResponse::Ok { data, .. } => return Ok(data),
                IpcResponse::IntentRequired(review) => {
                    let commit = IpcRequest::IntentCommit {
                        intent_id: review.intent_id,
                        proof: prove(&review),
                    };
                    answer = self.host.call(caller(), commit).await;
                }
                IpcResponse::Error { code, message, .. } => return Err(Refused { code, message }),
            }
        }
    }

    /// Send `request`, which the gate must hold for the user's review
    /// (`IntentRequired`); prove and commit it as [`call`](Self::call)
    /// does.
    pub async fn call_held(&self, request: IpcRequest) -> Result<serde_json::Value, Refused> {
        let name = format!("{request:?}");
        let IpcResponse::IntentRequired(review) = self.host.call(caller(), request).await else {
            panic!("node {}: {name} was not held for review", self.index);
        };
        self.call(IpcRequest::IntentCommit {
            intent_id: review.intent_id,
            proof: prove(&review),
        })
        .await
    }

    /// The events this host publishes from now on, as a frontend
    /// subscribed to its journal sees them (`HostApi::subscribe`).
    pub async fn events(&self) -> Events {
        let mut stream = self.host.subscribe(None).await.expect("the event stream");
        let seen = Events::default();
        let sink = Arc::clone(&seen.0);
        tokio::spawn(async move {
            while let Some(frame) = stream.next().await {
                if let StreamFrame::Event(sequenced) = frame {
                    sink.lock().push(sequenced.event);
                }
            }
        });
        seen
    }

    /// Whether any event seen so far satisfies `pred`.
    pub fn saw(events: &Events, pred: impl Fn(&SubscriptionEvent) -> bool) -> bool {
        events.0.lock().iter().any(pred)
    }

    /// The unlocked identity's `PrekeyStatus`, as the CLI's `keys prekeys
    /// status` and the debug `pqxdh_bundle_info` read it.
    pub async fn prekey_status(&self) -> rekindle_types::dto::identity::PrekeyStatus {
        serde_json::from_value(self.ok(IpcRequest::PrekeyStatus).await).expect("a prekey status")
    }

    /// [`call`](Self::call), which must succeed.
    pub async fn ok(&self, request: IpcRequest) -> serde_json::Value {
        let name = format!("{request:?}");
        match self.call(request).await {
            Ok(data) => data,
            Err(Refused { code, message }) => {
                panic!("node {}: {name} refused ({code}): {message}", self.index)
            }
        }
    }

    /// Create an identity named `name`; the create unlocks it. Returns
    /// once the unlock's own-records stage holds its records (the prekey
    /// record filled, then the profile naming it), as a user waits for
    /// their profile before adding a friend.
    pub async fn unlock_new_identity(&mut self, name: &str) {
        let created = self
            .ok(IpcRequest::IdentityCreate {
                display_name: Some(name.into()),
                passphrase: PASSPHRASE.into(),
            })
            .await;
        let view: rekindle_types::dto::identity::IdentityView =
            serde_json::from_value(created).expect("the opened identity");
        self.public_key = Some(view.public_key);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        while !self.records_held().await {
            assert!(
                tokio::time::Instant::now() < deadline,
                "node {}: own records not held",
                self.index
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Whether the unlocked identity's profile and mailbox are published.
    async fn records_held(&self) -> bool {
        let shown = self.ok(IpcRequest::IdentityShow).await;
        ["profileDhtKey", "mailboxDhtKey"]
            .iter()
            .all(|field| shown[field].as_str().is_some_and(|key| !key.is_empty()))
    }
}

/// What a host published since [`SimNode::events`] subscribed.
#[derive(Clone, Default)]
pub struct Events(Arc<parking_lot::Mutex<Vec<SubscriptionEvent>>>);

impl Events {
    /// The friend events, in order.
    pub fn friend(&self) -> Vec<FriendEvent> {
        self.0
            .lock()
            .iter()
            .filter_map(|event| match event {
                SubscriptionEvent::Friend(event) => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    /// How many friend events about `peer` match `pick`.
    pub fn count(&self, peer: &str, pick: fn(&FriendEvent) -> Option<&str>) -> usize {
        self.friend()
            .iter()
            .filter(|event| pick(event) == Some(peer))
            .count()
    }

    /// Whether any published event matches `pick`.
    pub fn any(&self, pick: impl Fn(&SubscriptionEvent) -> bool) -> bool {
        self.0.lock().iter().any(pick)
    }
}

/// Wait (in simulated time) until `check` holds, up to `limit`.
pub async fn eventually<F, Fut>(sim: &Netsim, limit: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        sim.quiesce().await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// A community node 0 created, with every other node joined through an
/// invite of its own, after each joiner lists it.
pub struct Community {
    pub sim: Netsim,
    pub nodes: Vec<SimNode>,
    pub governance_key: String,
    /// The last invite link (its secrets carry the slot seed).
    pub link: String,
}

impl Community {
    /// `n` nodes with identities; node 0 creates "Netsim" and every other
    /// node joins.
    pub async fn of(n: usize, seed: u64) -> Self {
        let mut community = Self::create(n, seed).await;
        for i in 1..n {
            community.join(i).await;
        }
        community
    }

    /// `n` nodes with identities; node 0 creates "Netsim"; nobody joins.
    pub async fn create(n: usize, seed: u64) -> Self {
        let (sim, mut nodes) = start(n, seed).await;
        for (i, node) in nodes.iter_mut().enumerate() {
            node.unlock_new_identity(&format!("member{i}")).await;
        }
        let created = nodes[0]
            .ok(IpcRequest::CommunityCreate {
                name: "Netsim".into(),
                description: String::new(),
                approval_required: false,
            })
            .await;
        let governance_key = created["governance_key"]
            .as_str()
            .expect("a governance key")
            .to_string();
        Self {
            sim,
            nodes,
            governance_key,
            link: String::new(),
        }
    }

    /// A one-use invite from node 0; kept as [`link`](Self::link).
    pub async fn invite(&mut self) -> String {
        let invite = self.nodes[0]
            .ok(IpcRequest::InviteCreate {
                community: self.governance_key.clone(),
                max_uses: 1,
                expires_seconds: None,
            })
            .await;
        self.link = invite["invite_link"]
            .as_str()
            .expect("an invite link")
            .to_string();
        self.link.clone()
    }

    /// Node `i` joins through a fresh invite, and lists the community.
    pub async fn join(&mut self, i: usize) {
        let link = self.invite().await;
        self.nodes[i]
            .ok(IpcRequest::CommunityJoin { invite: link })
            .await;
        let listed = eventually(&self.sim, Duration::from_secs(60), || async {
            self.nodes[i]
                .call(IpcRequest::CommunityList)
                .await
                .is_ok_and(|v| v.to_string().contains(&self.governance_key))
        })
        .await;
        assert!(listed, "node {i} lists the community");
    }

    /// Node `i`'s community info.
    pub async fn info(&self, i: usize) -> serde_json::Value {
        self.nodes[i]
            .ok(IpcRequest::CommunityInfo {
                governance_key: self.governance_key.clone(),
            })
            .await
    }

    /// Node `i`'s pseudonym in the community.
    pub async fn pseudonym(&self, i: usize) -> String {
        self.info(i).await["our_pseudonym"]
            .as_str()
            .expect("a pseudonym")
            .to_string()
    }

    /// Whether node `i` sees itself holding `role_id` within a minute.
    pub async fn holds_role(&self, i: usize, role_id: u32) -> bool {
        eventually(&self.sim, Duration::from_secs(60), || async {
            self.info(i).await["our_roles"]
                .as_array()
                .is_some_and(|roles| roles.iter().any(|r| r.as_u64() == Some(u64::from(role_id))))
        })
        .await
    }

    /// Post `body` from node `i` to the general channel.
    pub async fn post(&self, i: usize, body: &str) -> Result<serde_json::Value, Refused> {
        self.nodes[i]
            .call(IpcRequest::ChannelSend {
                community: self.governance_key.clone(),
                channel: "general".into(),
                body: body.into(),
                reply_to: None,
            })
            .await
    }

    /// Whether node `i` reads `body` in the general channel within `limit`.
    pub async fn reads(&self, i: usize, body: &str, limit: Duration) -> bool {
        eventually(&self.sim, limit, || async {
            self.nodes[i]
                .call(IpcRequest::ChannelHistory {
                    community: self.governance_key.clone(),
                    channel: "general".into(),
                    limit: 100,
                })
                .await
                .is_ok_and(|v| v.to_string().contains(body))
        })
        .await
    }
}
