//! Client synced actor properties of the vanilla Bedrock entities.
//!
//! The vanilla resource pack reads them with `query.property`: a pig picks its texture
//! from `minecraft:climate_variant`, a cow its sounds from `minecraft:sound_variant`.
//! A client learns the definitions only from `SyncActorProperty`, and an `AddActor`
//! addresses the values by the position of a property in that definition.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::bedrock::client::set_actor_data::PropertySyncData;
use pumpkin_protocol::bedrock::client::sync_actor_property::CSyncActorProperty;
use serde::Deserialize;

/// The temperature variant of a pig, cow or chicken.
pub const CLIMATE_VARIANT: &str = "minecraft:climate_variant";
/// The sound set of a pig, cow, chicken, cat or wolf.
pub const SOUND_VARIANT: &str = "minecraft:sound_variant";

static DEFINITIONS: LazyLock<BTreeMap<String, Vec<PropertyDefinition>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../assets/bedrock/entity_properties.json"
    ))
    .unwrap_or_else(|error| {
        tracing::error!("Failed to read the Bedrock entity properties: {error}");
        BTreeMap::new()
    })
});

static DEFINITION_PACKETS: LazyLock<Vec<CSyncActorProperty>> = LazyLock::new(|| {
    DEFINITIONS
        .iter()
        .map(|(identifier, definitions)| {
            let mut property_data = NbtCompound::new();
            property_data.put_string("type", identifier.clone());
            property_data.put_list(
                "properties",
                definitions
                    .iter()
                    .map(|definition| NbtTag::Compound(definition.to_nbt()))
                    .collect(),
            );
            CSyncActorProperty { property_data }
        })
        .collect()
});

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum PropertyKind {
    Int {
        range: [i32; 2],
        default: i32,
    },
    Float {
        range: [f32; 2],
        default: f32,
    },
    Bool {
        default: bool,
    },
    Enum {
        values: Vec<String>,
        default: String,
    },
}

#[derive(Deserialize)]
struct PropertyDefinition {
    name: String,
    #[serde(flatten)]
    kind: PropertyKind,
}

impl PropertyDefinition {
    fn to_nbt(&self) -> NbtCompound {
        let mut nbt = NbtCompound::new();
        nbt.put_string("name", self.name.clone());
        match &self.kind {
            PropertyKind::Int {
                range: [min, max], ..
            } => {
                nbt.put_int("type", 0);
                nbt.put_int("min", *min);
                nbt.put_int("max", *max);
            }
            PropertyKind::Float {
                range: [min, max], ..
            } => {
                nbt.put_int("type", 1);
                nbt.put_float("min", *min);
                nbt.put_float("max", *max);
            }
            PropertyKind::Bool { .. } => nbt.put_int("type", 2),
            PropertyKind::Enum { values, .. } => {
                nbt.put_int("type", 3);
                nbt.put_list(
                    "enum",
                    values
                        .iter()
                        .map(|value| NbtTag::String(value.as_str().into()))
                        .collect(),
                );
            }
        }
        nbt
    }
}

/// One packet for each entity type with client synced properties.
#[must_use]
pub fn definition_packets() -> &'static [CSyncActorProperty] {
    &DEFINITION_PACKETS
}

/// The values an `AddActor` carries for `identifier`.
///
/// `chosen` names an enum value by property name. Everything else gets the default of its
/// definition, and so does a value this client version does not define, which is what a
/// variant from a datapack is.
#[must_use]
pub fn spawn_values(identifier: &str, chosen: &[(&str, &str)]) -> PropertySyncData {
    let mut values = PropertySyncData::default();
    let Some(definitions) = DEFINITIONS.get(identifier) else {
        return values;
    };
    for (index, definition) in (0u32..).zip(definitions) {
        match &definition.kind {
            PropertyKind::Int { default, .. } => {
                values.int_entries_list.insert(index, *default);
            }
            PropertyKind::Float { default, .. } => {
                values.float_entries_list.insert(index, *default);
            }
            PropertyKind::Bool { default } => {
                values.int_entries_list.insert(index, i32::from(*default));
            }
            PropertyKind::Enum {
                values: names,
                default,
            } => {
                let position = |wanted: &str| names.iter().position(|name| name == wanted);
                let selected = chosen
                    .iter()
                    .find(|(name, _)| *name == definition.name)
                    .and_then(|(_, value)| position(value))
                    .or_else(|| position(default))
                    .unwrap_or(0);
                values.int_entries_list.insert(index, selected as i32);
            }
        }
    }
    values
}

/// The Bedrock value of a Java sound variant. Only these two are named differently.
#[must_use]
pub fn sound_variant_value(java_name: &'static str) -> &'static str {
    match java_name {
        "classic" => "default",
        "angry" => "mad",
        other => other,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn enum_names(identifier: &str, property: &str) -> Vec<String> {
        let definition = DEFINITIONS[identifier]
            .iter()
            .find(|definition| definition.name == property)
            .expect("the property is defined");
        match &definition.kind {
            PropertyKind::Enum { values, .. } => values.clone(),
            _ => panic!("{property} is not an enum"),
        }
    }

    fn property_nbt(identifier: &str, index: usize) -> &'static NbtCompound {
        let packet = definition_packets()
            .iter()
            .find(|packet| packet.property_data.get_string("type") == Some(identifier))
            .expect("the entity has properties");
        match &packet.property_data.get_list("properties").unwrap()[index] {
            NbtTag::Compound(property) => property,
            _ => panic!("a definition is a compound"),
        }
    }

    #[test]
    fn every_enum_default_is_one_of_its_values() {
        for definitions in DEFINITIONS.values() {
            for definition in definitions {
                if let PropertyKind::Enum { values, default } = &definition.kind {
                    assert!(values.contains(default), "{}", definition.name);
                }
            }
        }
    }

    #[test]
    fn definitions_use_the_wire_format_of_the_client() {
        let has_nectar = property_nbt("minecraft:bee", 0);
        assert_eq!(has_nectar.get_string("name"), Some("minecraft:has_nectar"));
        assert_eq!(has_nectar.get_int("type"), Some(2));

        let swaying_ticks = property_nbt("minecraft:creaking", 1);
        assert_eq!(swaying_ticks.get_int("type"), Some(0));
        assert_eq!(swaying_ticks.get_int("min"), Some(0));
        assert_eq!(swaying_ticks.get_int("max"), Some(6));

        let climate = property_nbt("minecraft:pig", 0);
        assert_eq!(climate.get_int("type"), Some(3));
        let names: Vec<_> = climate
            .get_list("enum")
            .unwrap()
            .iter()
            .filter_map(NbtTag::extract_string)
            .collect();
        assert_eq!(names, ["temperate", "warm", "cold"]);
    }

    #[test]
    fn a_chosen_value_is_sent_by_its_position_in_the_definition() {
        let values = spawn_values(
            "minecraft:pig",
            &[(CLIMATE_VARIANT, "cold"), (SOUND_VARIANT, "mini")],
        );
        assert_eq!(values.int_entries_list, HashMap::from([(0, 2), (1, 2)]));
        assert!(values.float_entries_list.is_empty());
    }

    #[test]
    fn what_is_not_chosen_gets_its_default() {
        let values = spawn_values("minecraft:cow", &[(SOUND_VARIANT, "moody")]);
        assert_eq!(values.int_entries_list, HashMap::from([(0, 0), (1, 1)]));

        let ghast = spawn_values("minecraft:happy_ghast", &[]);
        assert_eq!(ghast.int_entries_list, HashMap::from([(0, 1)]));
    }

    #[test]
    fn a_value_the_client_does_not_define_falls_back_to_the_default() {
        let values = spawn_values("minecraft:cow", &[(CLIMATE_VARIANT, "from_a_datapack")]);
        assert_eq!(values.int_entries_list[&0], 0);
    }

    #[test]
    fn a_property_the_entity_does_not_have_is_ignored() {
        let values = spawn_values("minecraft:bee", &[(CLIMATE_VARIANT, "warm")]);
        assert_eq!(values.int_entries_list, HashMap::from([(0, 0)]));
    }

    #[test]
    fn an_entity_without_properties_sends_none() {
        let values = spawn_values("minecraft:zombie", &[(CLIMATE_VARIANT, "warm")]);
        assert!(values.int_entries_list.is_empty());
        assert!(values.float_entries_list.is_empty());
    }

    #[test]
    fn java_sound_variants_map_onto_the_bedrock_names() {
        for (java, identifier) in [
            ("angry", "minecraft:wolf"),
            ("big", "minecraft:wolf"),
            ("classic", "minecraft:wolf"),
            ("cute", "minecraft:wolf"),
            ("grumpy", "minecraft:wolf"),
            ("puglin", "minecraft:wolf"),
            ("sad", "minecraft:wolf"),
            ("big", "minecraft:pig"),
            ("classic", "minecraft:pig"),
            ("mini", "minecraft:pig"),
            ("classic", "minecraft:cow"),
            ("moody", "minecraft:cow"),
            ("classic", "minecraft:chicken"),
            ("picky", "minecraft:chicken"),
            ("classic", "minecraft:cat"),
            ("royal", "minecraft:cat"),
        ] {
            let names = enum_names(identifier, SOUND_VARIANT);
            assert!(
                names.contains(&sound_variant_value(java).to_string()),
                "{java} on {identifier}"
            );
        }
    }
}
