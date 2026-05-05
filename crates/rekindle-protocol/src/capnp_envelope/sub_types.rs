//! Leaf type encoders / decoders shared across `control.rs` and
//! `governance.rs`. Each pair (`write_<T>` / `read_<T>`) maps a Rust
//! domain type to its Cap'n Proto schema.
//!
//! Helpers here live in one module so adding a new variant in either
//! `ControlPayload` or `GovernanceEntry` only requires *one* place to
//! update if a new sub-type is referenced.

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::dht::community::envelope::{
    OnboardingAnswer, PresenceGameInfo, VoiceRosterEntry,
};
use crate::dht::community::types::MemberSummary;
use crate::error::ProtocolError;
use rekindle_types::event::{
    DayOfWeek, EventInfo, EventLocation, EventRsvp, RecurrenceFrequency, RecurrenceRule,
};
use rekindle_types::game_server::GameServerInfo;
use rekindle_types::id::{CategoryId, ChannelId, EventId, PseudonymKey, RoleId, ThreadId};
use rekindle_types::member::MemberInfo;
use rekindle_types::mek::ChannelMekDelivery;
use rekindle_types::message::{BootstrapChannelMessages, BootstrapMessage, SyncedMessage};
use rekindle_types::thread::ThreadInfo;

use super::len_u32;

// ── PresenceGameInfo (envelope.PresenceUpdate.game_info) ─────────────

pub(super) fn write_presence_game_info(
    mut b: crate::community_envelope_capnp::presence_game_info::Builder<'_>,
    g: &PresenceGameInfo,
) {
    b.set_game_name(&g.game_name);
    b.set_has_game_id(g.game_id.is_some());
    if let Some(id) = g.game_id {
        b.set_game_id(id);
    }
    b.set_has_elapsed_secs(g.elapsed_seconds.is_some());
    if let Some(s) = g.elapsed_seconds {
        b.set_elapsed_seconds(s);
    }
    b.set_has_server_address(g.server_address.is_some());
    if let Some(ref addr) = g.server_address {
        b.set_server_address(addr);
    }
}

pub(super) fn read_presence_game_info(
    r: crate::community_envelope_capnp::presence_game_info::Reader<'_>,
) -> Result<PresenceGameInfo, ProtocolError> {
    Ok(PresenceGameInfo {
        game_name: text_to_string(r.get_game_name().map_err(|e| capnp_err(&e))?)?,
        game_id: if r.get_has_game_id() {
            Some(r.get_game_id())
        } else {
            None
        },
        elapsed_seconds: if r.get_has_elapsed_secs() {
            Some(r.get_elapsed_seconds())
        } else {
            None
        },
        server_address: if r.get_has_server_address() {
            Some(text_to_string(
                r.get_server_address().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
    })
}

// ── MemberInfo ───────────────────────────────────────────────────────

pub(super) fn write_member_info(
    mut b: crate::community_member_capnp::member_info::Builder<'_>,
    m: &MemberInfo,
) {
    b.set_pseudonym_key(&m.pseudonym_key);
    b.set_display_name(&m.display_name);
    let mut role_ids = b.reborrow().init_role_ids(len_u32(m.role_ids.len()));
    for (i, id) in m.role_ids.iter().enumerate() {
        role_ids.set(len_u32(i), *id);
    }
    b.set_status(&m.status);
    b.set_timeout_until(m.timeout_until.unwrap_or(0));
    if let Some(ref blob) = m.route_blob {
        b.set_route_blob(blob);
    }
    if let Some(ref bio) = m.bio {
        b.set_bio(bio);
    }
    if let Some(ref pr) = m.pronouns {
        b.set_pronouns(pr);
    }
    b.set_theme_color(m.theme_color.unwrap_or(0));
    let mut badges = b.reborrow().init_badges(len_u32(m.badges.len()));
    for (i, badge) in m.badges.iter().enumerate() {
        badges.set(len_u32(i), badge.as_str());
    }
    b.set_last_seen(m.last_seen);
}

pub(super) fn read_member_info(
    r: crate::community_member_capnp::member_info::Reader<'_>,
) -> Result<MemberInfo, ProtocolError> {
    let role_ids: Vec<u32> = r
        .get_role_ids()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .collect();
    let route_blob_bytes = r.get_route_blob().map_err(|e| capnp_err(&e))?;
    let bio_text = text_to_string(r.get_bio().map_err(|e| capnp_err(&e))?)?;
    let pronouns_text = text_to_string(r.get_pronouns().map_err(|e| capnp_err(&e))?)?;
    let badges: Result<Vec<String>, ProtocolError> = r
        .get_badges()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect();
    let timeout = r.get_timeout_until();
    let theme = r.get_theme_color();
    Ok(MemberInfo {
        pseudonym_key: text_to_string(r.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        display_name: text_to_string(r.get_display_name().map_err(|e| capnp_err(&e))?)?,
        role_ids,
        status: text_to_string(r.get_status().map_err(|e| capnp_err(&e))?)?,
        timeout_until: if timeout == 0 { None } else { Some(timeout) },
        route_blob: if route_blob_bytes.is_empty() {
            None
        } else {
            Some(route_blob_bytes.to_vec())
        },
        bio: if bio_text.is_empty() {
            None
        } else {
            Some(bio_text)
        },
        pronouns: if pronouns_text.is_empty() {
            None
        } else {
            Some(pronouns_text)
        },
        theme_color: if theme == 0 { None } else { Some(theme) },
        badges: badges?,
        last_seen: r.get_last_seen(),
    })
}

// ── ThreadInfo ───────────────────────────────────────────────────────

pub(super) fn write_thread_info(
    mut b: crate::community_thread_capnp::thread_info::Builder<'_>,
    t: &ThreadInfo,
) {
    b.set_id(&t.id);
    b.set_channel_id(&t.channel_id);
    b.set_name(&t.name);
    b.set_starter_message_id(&t.starter_message_id);
    b.set_creator_pseudonym(&t.creator_pseudonym);
    if let Some(ref tag) = t.forum_tag {
        b.set_forum_tag(tag);
    }
    b.set_created_at(t.created_at);
    b.set_archived(t.archived);
    b.set_auto_archive_seconds(t.auto_archive_seconds);
    b.set_last_message_at(t.last_message_at);
    b.set_message_count(t.message_count);
}

pub(super) fn read_thread_info(
    r: crate::community_thread_capnp::thread_info::Reader<'_>,
) -> Result<ThreadInfo, ProtocolError> {
    let forum_tag_text = text_to_string(r.get_forum_tag().map_err(|e| capnp_err(&e))?)?;
    Ok(ThreadInfo {
        id: text_to_string(r.get_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: text_to_string(r.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(r.get_name().map_err(|e| capnp_err(&e))?)?,
        starter_message_id: text_to_string(
            r.get_starter_message_id().map_err(|e| capnp_err(&e))?,
        )?,
        creator_pseudonym: text_to_string(r.get_creator_pseudonym().map_err(|e| capnp_err(&e))?)?,
        forum_tag: if forum_tag_text.is_empty() {
            None
        } else {
            Some(forum_tag_text)
        },
        created_at: r.get_created_at(),
        archived: r.get_archived(),
        auto_archive_seconds: r.get_auto_archive_seconds(),
        last_message_at: r.get_last_message_at(),
        message_count: r.get_message_count(),
    })
}

// ── GameServerInfo ───────────────────────────────────────────────────

pub(super) fn write_game_server_info(
    mut b: crate::community_game_server_capnp::game_server_info::Builder<'_>,
    g: &GameServerInfo,
) {
    b.set_id(&g.id);
    b.set_game_id(&g.game_id);
    b.set_label(&g.label);
    b.set_address(&g.address);
    b.set_added_by(&g.added_by);
    b.set_created_at(g.created_at);
}

pub(super) fn read_game_server_info(
    r: crate::community_game_server_capnp::game_server_info::Reader<'_>,
) -> Result<GameServerInfo, ProtocolError> {
    Ok(GameServerInfo {
        id: text_to_string(r.get_id().map_err(|e| capnp_err(&e))?)?,
        game_id: text_to_string(r.get_game_id().map_err(|e| capnp_err(&e))?)?,
        label: text_to_string(r.get_label().map_err(|e| capnp_err(&e))?)?,
        address: text_to_string(r.get_address().map_err(|e| capnp_err(&e))?)?,
        added_by: text_to_string(r.get_added_by().map_err(|e| capnp_err(&e))?)?,
        created_at: r.get_created_at(),
    })
}

// ── ChannelMekDelivery ───────────────────────────────────────────────

pub(super) fn write_channel_mek_delivery(
    mut b: crate::community_mek_capnp::channel_mek_delivery::Builder<'_>,
    d: &ChannelMekDelivery,
) {
    if let Some(ref id) = d.channel_id {
        b.set_channel_id(id);
    }
    b.set_generation(d.generation);
    b.set_wrapped_mek(&d.wrapped_mek);
}

pub(super) fn read_channel_mek_delivery(
    r: crate::community_mek_capnp::channel_mek_delivery::Reader<'_>,
) -> Result<ChannelMekDelivery, ProtocolError> {
    let channel_id_text = text_to_string(r.get_channel_id().map_err(|e| capnp_err(&e))?)?;
    Ok(ChannelMekDelivery {
        channel_id: if channel_id_text.is_empty() {
            None
        } else {
            Some(channel_id_text)
        },
        generation: r.get_generation(),
        wrapped_mek: r.get_wrapped_mek().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

// ── BootstrapMessage / BootstrapChannelMessages ──────────────────────

fn write_bootstrap_message(
    mut b: crate::community_message_capnp::bootstrap_message::Builder<'_>,
    m: &BootstrapMessage,
) {
    b.set_message_id(&m.message_id);
    b.set_sender_pseudonym(&m.sender_pseudonym);
    b.set_ciphertext(&m.ciphertext);
    b.set_mek_generation(m.mek_generation);
    b.set_timestamp(m.timestamp);
}

fn read_bootstrap_message(
    r: crate::community_message_capnp::bootstrap_message::Reader<'_>,
) -> Result<BootstrapMessage, ProtocolError> {
    Ok(BootstrapMessage {
        message_id: text_to_string(r.get_message_id().map_err(|e| capnp_err(&e))?)?,
        sender_pseudonym: text_to_string(
            r.get_sender_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        ciphertext: r.get_ciphertext().map_err(|e| capnp_err(&e))?.to_vec(),
        mek_generation: r.get_mek_generation(),
        timestamp: r.get_timestamp(),
    })
}

pub(super) fn write_bootstrap_channel_messages(
    mut b: crate::community_message_capnp::bootstrap_channel_messages::Builder<'_>,
    g: &BootstrapChannelMessages,
) {
    b.set_channel_id(&g.channel_id);
    let mut list = b.reborrow().init_messages(len_u32(g.messages.len()));
    for (i, msg) in g.messages.iter().enumerate() {
        write_bootstrap_message(list.reborrow().get(len_u32(i)), msg);
    }
}

pub(super) fn read_bootstrap_channel_messages(
    r: crate::community_message_capnp::bootstrap_channel_messages::Reader<'_>,
) -> Result<BootstrapChannelMessages, ProtocolError> {
    let messages: Result<Vec<BootstrapMessage>, ProtocolError> = r
        .get_messages()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(read_bootstrap_message)
        .collect();
    Ok(BootstrapChannelMessages {
        channel_id: text_to_string(r.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        messages: messages?,
    })
}

// ── SyncedMessage ────────────────────────────────────────────────────

pub(super) fn write_synced_message(
    mut b: crate::community_message_capnp::synced_message::Builder<'_>,
    m: &SyncedMessage,
) {
    b.set_sender_key(&m.sender_key);
    b.set_body(&m.body);
    b.set_timestamp(m.timestamp);
    b.set_has_mek_generation(m.mek_generation.is_some());
    if let Some(g) = m.mek_generation {
        b.set_mek_generation(g);
    }
    b.set_has_lamport_ts(m.lamport_ts.is_some());
    if let Some(l) = m.lamport_ts {
        b.set_lamport_ts(l);
    }
}

pub(super) fn read_synced_message(
    r: crate::community_message_capnp::synced_message::Reader<'_>,
) -> Result<SyncedMessage, ProtocolError> {
    Ok(SyncedMessage {
        sender_key: text_to_string(r.get_sender_key().map_err(|e| capnp_err(&e))?)?,
        body: text_to_string(r.get_body().map_err(|e| capnp_err(&e))?)?,
        timestamp: r.get_timestamp(),
        mek_generation: if r.get_has_mek_generation() {
            Some(r.get_mek_generation())
        } else {
            None
        },
        lamport_ts: if r.get_has_lamport_ts() {
            Some(r.get_lamport_ts())
        } else {
            None
        },
    })
}

// ── EventInfo + nested RecurrenceRule / EventLocation ────────────────

pub(super) fn write_recurrence_rule_via_event_capnp(
    b: crate::community_event_capnp::recurrence_rule::Builder<'_>,
    r: &RecurrenceRule,
) {
    write_recurrence_rule(b, r);
}

pub(super) fn read_recurrence_rule_via_event_capnp(
    r: crate::community_event_capnp::recurrence_rule::Reader<'_>,
) -> Result<RecurrenceRule, ProtocolError> {
    read_recurrence_rule(r)
}

pub(super) fn write_event_location_via_event_capnp(
    b: crate::community_event_capnp::event_location::Builder<'_>,
    l: &EventLocation,
) {
    write_event_location(b, l);
}

pub(super) fn read_event_location_via_event_capnp(
    r: crate::community_event_capnp::event_location::Reader<'_>,
) -> Result<EventLocation, ProtocolError> {
    read_event_location(r)
}

fn write_recurrence_rule(
    mut b: crate::community_event_capnp::recurrence_rule::Builder<'_>,
    r: &RecurrenceRule,
) {
    use crate::community_event_capnp::RecurrenceFrequency as Cap;
    b.set_frequency(match r.frequency {
        RecurrenceFrequency::Daily => Cap::Daily,
        RecurrenceFrequency::Weekly => Cap::Weekly,
        RecurrenceFrequency::Monthly => Cap::Monthly,
    });
    b.set_interval(r.interval);
    if let Some(ref days) = r.days_of_week {
        let mut list = b.reborrow().init_days_of_week(len_u32(days.len()));
        for (i, d) in days.iter().enumerate() {
            list.set(
                len_u32(i),
                match d {
                    DayOfWeek::Sunday => crate::community_event_capnp::DayOfWeek::Sunday,
                    DayOfWeek::Monday => crate::community_event_capnp::DayOfWeek::Monday,
                    DayOfWeek::Tuesday => crate::community_event_capnp::DayOfWeek::Tuesday,
                    DayOfWeek::Wednesday => crate::community_event_capnp::DayOfWeek::Wednesday,
                    DayOfWeek::Thursday => crate::community_event_capnp::DayOfWeek::Thursday,
                    DayOfWeek::Friday => crate::community_event_capnp::DayOfWeek::Friday,
                    DayOfWeek::Saturday => crate::community_event_capnp::DayOfWeek::Saturday,
                },
            );
        }
    }
    b.set_has_until(r.until.is_some());
    if let Some(u) = r.until {
        b.set_until(u);
    }
    b.set_has_count(r.count.is_some());
    if let Some(c) = r.count {
        b.set_count(c);
    }
}

fn read_recurrence_rule(
    r: crate::community_event_capnp::recurrence_rule::Reader<'_>,
) -> Result<RecurrenceRule, ProtocolError> {
    use crate::capnp_codec::not_in_schema;
    use crate::community_event_capnp::DayOfWeek as CapDay;
    use crate::community_event_capnp::RecurrenceFrequency as Cap;
    let frequency = match r.get_frequency().map_err(not_in_schema)? {
        Cap::Daily => RecurrenceFrequency::Daily,
        Cap::Weekly => RecurrenceFrequency::Weekly,
        Cap::Monthly => RecurrenceFrequency::Monthly,
    };
    let days_list = r.get_days_of_week().map_err(|e| capnp_err(&e))?;
    let days_of_week = if days_list.is_empty() {
        None
    } else {
        let mut out = Vec::with_capacity(days_list.len() as usize);
        for d in days_list {
            out.push(match d.map_err(not_in_schema)? {
                CapDay::Sunday => DayOfWeek::Sunday,
                CapDay::Monday => DayOfWeek::Monday,
                CapDay::Tuesday => DayOfWeek::Tuesday,
                CapDay::Wednesday => DayOfWeek::Wednesday,
                CapDay::Thursday => DayOfWeek::Thursday,
                CapDay::Friday => DayOfWeek::Friday,
                CapDay::Saturday => DayOfWeek::Saturday,
            });
        }
        Some(out)
    };
    Ok(RecurrenceRule {
        frequency,
        interval: r.get_interval(),
        days_of_week,
        until: if r.get_has_until() {
            Some(r.get_until())
        } else {
            None
        },
        count: if r.get_has_count() {
            Some(r.get_count())
        } else {
            None
        },
    })
}

fn write_event_location(
    mut b: crate::community_event_capnp::event_location::Builder<'_>,
    loc: &EventLocation,
) {
    match loc {
        EventLocation::VoiceChannel(id) => {
            b.set_voice_channel(hex::encode(id.0));
        }
        EventLocation::StageChannel(id) => {
            b.set_stage_channel(hex::encode(id.0));
        }
        EventLocation::External(url) => {
            b.set_external(url);
        }
        EventLocation::InGame {
            game_id,
            server_address,
        } => {
            let mut ig = b.init_in_game();
            ig.set_game_id(*game_id);
            if let Some(ref addr) = server_address {
                ig.set_server_address(addr);
            }
        }
    }
}

fn read_event_location(
    r: crate::community_event_capnp::event_location::Reader<'_>,
) -> Result<EventLocation, ProtocolError> {
    use crate::capnp_codec::not_in_schema;
    use crate::community_event_capnp::event_location::Which;
    match r.which().map_err(not_in_schema)? {
        Which::VoiceChannel(t) => {
            let s = text_to_string(t.map_err(|e| capnp_err(&e))?)?;
            Ok(EventLocation::VoiceChannel(parse_channel_id(&s)?))
        }
        Which::StageChannel(t) => {
            let s = text_to_string(t.map_err(|e| capnp_err(&e))?)?;
            Ok(EventLocation::StageChannel(parse_channel_id(&s)?))
        }
        Which::External(t) => Ok(EventLocation::External(text_to_string(
            t.map_err(|e| capnp_err(&e))?,
        )?)),
        Which::InGame(g) => {
            let g = g.map_err(|e| capnp_err(&e))?;
            let server_address_text =
                text_to_string(g.get_server_address().map_err(|e| capnp_err(&e))?)?;
            Ok(EventLocation::InGame {
                game_id: g.get_game_id(),
                server_address: if server_address_text.is_empty() {
                    None
                } else {
                    Some(server_address_text)
                },
            })
        }
    }
}

fn parse_channel_id(hex_str: &str) -> Result<ChannelId, ProtocolError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| ProtocolError::Deserialization(format!("invalid channel id hex: {e}")))?;
    let arr: [u8; 16] = bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("channel id must be 16 bytes".into()))?;
    Ok(ChannelId(arr))
}

pub(super) fn write_event_info(
    mut b: crate::community_event_capnp::event_info::Builder<'_>,
    e: &EventInfo,
) {
    b.set_id(&e.id);
    b.set_title(&e.title);
    b.set_description(&e.description);
    b.set_creator_pseudonym(&e.creator_pseudonym);
    b.set_start_time(e.start_time);
    b.set_has_end_time(e.end_time.is_some());
    if let Some(t) = e.end_time {
        b.set_end_time(t);
    }
    if let Some(ref ch) = e.channel_id {
        b.set_channel_id(ch);
    }
    b.set_has_max_attendees(e.max_attendees.is_some());
    if let Some(m) = e.max_attendees {
        b.set_max_attendees(m);
    }
    b.set_created_at(e.created_at);
    b.set_status(&e.status);
    let mut rsvps = b.reborrow().init_rsvps(len_u32(e.rsvps.len()));
    for (i, rsvp) in e.rsvps.iter().enumerate() {
        let mut entry = rsvps.reborrow().get(len_u32(i));
        entry.set_pseudonym_key(&rsvp.pseudonym_key);
        entry.set_status(&rsvp.status);
    }
    if let Some(ref c) = e.cover_image_ref {
        b.set_cover_image_ref(c);
    }
    b.set_has_recurrence(e.recurrence.is_some());
    if let Some(ref r) = e.recurrence {
        write_recurrence_rule(b.reborrow().init_recurrence(), r);
    }
    b.set_has_location(e.location.is_some());
    if let Some(ref loc) = e.location {
        write_event_location(b.reborrow().init_location(), loc);
    }
}

pub(super) fn read_event_info(
    r: crate::community_event_capnp::event_info::Reader<'_>,
) -> Result<EventInfo, ProtocolError> {
    let channel_id_text = text_to_string(r.get_channel_id().map_err(|e| capnp_err(&e))?)?;
    let cover_text = text_to_string(r.get_cover_image_ref().map_err(|e| capnp_err(&e))?)?;
    let rsvps: Result<Vec<EventRsvp>, ProtocolError> = r
        .get_rsvps()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|rsvp| {
            Ok(EventRsvp {
                pseudonym_key: text_to_string(
                    rsvp.get_pseudonym_key().map_err(|e| capnp_err(&e))?,
                )?,
                status: text_to_string(rsvp.get_status().map_err(|e| capnp_err(&e))?)?,
            })
        })
        .collect();
    let recurrence = if r.get_has_recurrence() {
        Some(read_recurrence_rule(
            r.get_recurrence().map_err(|e| capnp_err(&e))?,
        )?)
    } else {
        None
    };
    let location = if r.get_has_location() {
        Some(read_event_location(
            r.get_location().map_err(|e| capnp_err(&e))?,
        )?)
    } else {
        None
    };
    Ok(EventInfo {
        id: text_to_string(r.get_id().map_err(|e| capnp_err(&e))?)?,
        title: text_to_string(r.get_title().map_err(|e| capnp_err(&e))?)?,
        description: text_to_string(r.get_description().map_err(|e| capnp_err(&e))?)?,
        creator_pseudonym: text_to_string(
            r.get_creator_pseudonym().map_err(|e| capnp_err(&e))?,
        )?,
        start_time: r.get_start_time(),
        end_time: if r.get_has_end_time() {
            Some(r.get_end_time())
        } else {
            None
        },
        channel_id: if channel_id_text.is_empty() {
            None
        } else {
            Some(channel_id_text)
        },
        max_attendees: if r.get_has_max_attendees() {
            Some(r.get_max_attendees())
        } else {
            None
        },
        created_at: r.get_created_at(),
        status: text_to_string(r.get_status().map_err(|e| capnp_err(&e))?)?,
        rsvps: rsvps?,
        cover_image_ref: if cover_text.is_empty() {
            None
        } else {
            Some(cover_text)
        },
        recurrence,
        location,
    })
}

// ── MemberSummary ────────────────────────────────────────────────────

pub(super) fn write_member_summary(
    mut b: crate::community_envelope_capnp::member_summary::Builder<'_>,
    m: &MemberSummary,
) {
    b.set_pseudonym_key(&m.pseudonym_key);
    b.set_display_name(&m.display_name);
    let mut roles = b.reborrow().init_role_ids(len_u32(m.role_ids.len()));
    for (i, id) in m.role_ids.iter().enumerate() {
        roles.set(len_u32(i), *id);
    }
    b.set_joined_at(m.joined_at);
    b.set_subkey_index(m.subkey_index);
    b.set_onboarding_complete(m.onboarding_complete);
    b.set_has_timeout_until(m.timeout_until.is_some());
    if let Some(t) = m.timeout_until {
        b.set_timeout_until(t);
    }
}

pub(super) fn read_member_summary(
    r: crate::community_envelope_capnp::member_summary::Reader<'_>,
) -> Result<MemberSummary, ProtocolError> {
    let role_ids: Vec<u32> = r
        .get_role_ids()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .collect();
    Ok(MemberSummary {
        pseudonym_key: text_to_string(r.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        display_name: text_to_string(r.get_display_name().map_err(|e| capnp_err(&e))?)?,
        role_ids,
        joined_at: r.get_joined_at(),
        subkey_index: r.get_subkey_index(),
        onboarding_complete: r.get_onboarding_complete(),
        timeout_until: if r.get_has_timeout_until() {
            Some(r.get_timeout_until())
        } else {
            None
        },
    })
}

// ── VoiceRosterEntry ─────────────────────────────────────────────────

pub(super) fn write_voice_roster_entry(
    mut b: crate::community_envelope_capnp::voice_roster_entry::Builder<'_>,
    e: &VoiceRosterEntry,
) {
    b.set_pseudonym_key(&e.pseudonym_key);
    b.set_route_blob(&e.route_blob);
    b.set_muted(e.muted);
    b.set_deafened(e.deafened);
}

pub(super) fn read_voice_roster_entry(
    r: crate::community_envelope_capnp::voice_roster_entry::Reader<'_>,
) -> Result<VoiceRosterEntry, ProtocolError> {
    Ok(VoiceRosterEntry {
        pseudonym_key: text_to_string(r.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
        route_blob: r.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec(),
        muted: r.get_muted(),
        deafened: r.get_deafened(),
    })
}

// ── OnboardingAnswer ─────────────────────────────────────────────────

pub(super) fn write_onboarding_answer(
    mut b: crate::community_envelope_capnp::onboarding_answer::Builder<'_>,
    a: &OnboardingAnswer,
) {
    b.set_question_id(&a.question_id);
    let mut opts = b.reborrow().init_selected_options(len_u32(a.selected_options.len()));
    for (i, o) in a.selected_options.iter().enumerate() {
        opts.set(len_u32(i), o.as_str());
    }
}

pub(super) fn read_onboarding_answer(
    r: crate::community_envelope_capnp::onboarding_answer::Reader<'_>,
) -> Result<OnboardingAnswer, ProtocolError> {
    let opts: Result<Vec<String>, ProtocolError> = r
        .get_selected_options()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect();
    Ok(OnboardingAnswer {
        question_id: text_to_string(r.get_question_id().map_err(|e| capnp_err(&e))?)?,
        selected_options: opts?,
    })
}

// ── Typed-id helpers shared with governance.rs ───────────────────────

pub(super) fn pseudonym_key_to_capnp(
    mut b: crate::community_governance_capnp::pseudonym_key::Builder<'_>,
    p: &PseudonymKey,
) {
    b.set_bytes(&p.0);
}

pub(super) fn pseudonym_key_from_capnp(
    r: crate::community_governance_capnp::pseudonym_key::Reader<'_>,
) -> Result<PseudonymKey, ProtocolError> {
    let bytes = r.get_bytes().map_err(|e| capnp_err(&e))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("pseudonym key must be 32 bytes".into()))?;
    Ok(PseudonymKey(arr))
}

pub(super) fn uuid16_to_capnp(
    mut b: crate::community_governance_capnp::uuid16::Builder<'_>,
    bytes: &[u8; 16],
) {
    b.set_bytes(bytes);
}

pub(super) fn uuid16_from_capnp(
    r: crate::community_governance_capnp::uuid16::Reader<'_>,
) -> Result<[u8; 16], ProtocolError> {
    let bytes = r.get_bytes().map_err(|e| capnp_err(&e))?;
    bytes
        .try_into()
        .map_err(|_| ProtocolError::Deserialization("uuid16 must be 16 bytes".into()))
}

pub(super) fn channel_id_from_capnp(
    r: crate::community_governance_capnp::uuid16::Reader<'_>,
) -> Result<ChannelId, ProtocolError> {
    Ok(ChannelId(uuid16_from_capnp(r)?))
}

pub(super) fn role_id_from_capnp(
    r: crate::community_governance_capnp::uuid16::Reader<'_>,
) -> Result<RoleId, ProtocolError> {
    Ok(RoleId(uuid16_from_capnp(r)?))
}

pub(super) fn category_id_from_capnp(
    r: crate::community_governance_capnp::uuid16::Reader<'_>,
) -> Result<CategoryId, ProtocolError> {
    Ok(CategoryId(uuid16_from_capnp(r)?))
}

pub(super) fn thread_id_from_capnp(
    r: crate::community_governance_capnp::uuid16::Reader<'_>,
) -> Result<ThreadId, ProtocolError> {
    Ok(ThreadId(uuid16_from_capnp(r)?))
}

pub(super) fn event_id_from_capnp(
    r: crate::community_governance_capnp::uuid16::Reader<'_>,
) -> Result<EventId, ProtocolError> {
    Ok(EventId(uuid16_from_capnp(r)?))
}
