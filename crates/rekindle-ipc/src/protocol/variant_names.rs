//! Every `IpcRequest` leaf name has a `COMMAND_TABLE` row: the Host's gate
//! refuses a request missing one with 400. The names come from the derived
//! `Deserialize`, which hands its deserializer every variant, so a new
//! variant joins the check without anyone listing it.

use serde::de::{self, Deserialize, Deserializer, Visitor};

use super::{CommunityRequest, IpcRequest};

/// The variant names a derived enum `Deserialize` declared.
#[derive(Debug)]
struct Declared(&'static [&'static str]);

impl std::fmt::Display for Declared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} variants declared", self.0.len())
    }
}

impl std::error::Error for Declared {}

impl de::Error for Declared {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Self(&[])
    }
}

/// A deserializer that answers an enum's variant list as its error.
struct VariantNames;

impl<'de> Deserializer<'de> for VariantNames {
    type Error = Declared;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Declared> {
        Err(Declared(&[]))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        variants: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Declared> {
        Err(Declared(variants))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

fn variants<'de, T: Deserialize<'de>>() -> &'static [&'static str] {
    match T::deserialize(VariantNames) {
        Err(Declared(names)) => names,
        Ok(_) => panic!("an enum never deserializes from VariantNames"),
    }
}

#[test]
fn every_leaf_name_has_a_command_table_row() {
    let top = variants::<IpcRequest>();
    let community = variants::<CommunityRequest>();
    assert!(top.contains(&"Community"), "{top:?}");
    assert!(community.contains(&"ChannelSend"), "{community:?}");
    let missing: Vec<&str> = top
        .iter()
        .chain(community)
        .copied()
        .filter(|name| *name != "Community")
        .filter(|name| rekindle_types::intent::kind_of(name).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "requests with no COMMAND_TABLE row: {missing:?}"
    );
}

#[test]
fn a_community_leaf_keeps_its_name() {
    let request = IpcRequest::Community(CommunityRequest::CommunityList);
    assert_eq!(request.name(), "CommunityList");
    assert_eq!(request.lane(), super::Lane::Query);
}
