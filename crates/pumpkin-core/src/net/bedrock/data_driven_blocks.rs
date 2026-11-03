//! Definitions of the vanilla blocks Mojang made data driven.
//!
//! A client registers these blocks only from the `block_properties` of `StartGame`. Without
//! them it has no such block, no icon for its item, and a chunk can name blocks it does not know.

use std::io::Cursor;
use std::sync::LazyLock;

use pumpkin_nbt::nbt_compress::read_gzip_compound_tag;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::bedrock::client::start_game::BlockProperty;

static BLOCK_PROPERTIES: LazyLock<Vec<BlockProperty>> = LazyLock::new(|| {
    let asset = include_bytes!("../../../../../assets/bedrock/data_driven_blocks.nbt");
    let definitions = match read_gzip_compound_tag(Cursor::new(asset)) {
        Ok(definitions) => definitions,
        Err(error) => {
            tracing::error!("Failed to read the Bedrock data driven blocks: {error}");
            return Vec::new();
        }
    };
    let mut blocks: Vec<_> = definitions
        .child_tags
        .into_iter()
        .filter_map(|(name, definition)| match definition {
            NbtTag::Compound(properties) => Some(BlockProperty::new(name, properties)),
            _ => None,
        })
        .collect();
    blocks.sort_by(|left, right| left.name.cmp(&right.name));
    blocks
});

/// The `block_properties` of `StartGame`.
#[must_use]
pub fn block_properties() -> Vec<BlockProperty> {
    BLOCK_PROPERTIES.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_asset_defines_the_blocks_a_client_cannot_know_otherwise() {
        let blocks = block_properties();
        for name in [
            "minecraft:shelf_mushroom",
            "minecraft:red_shrub",
            "minecraft:white_wool_stairs",
            "minecraft:pink_concrete_double_slab",
        ] {
            let block = blocks.iter().find(|block| block.name == name);
            assert!(
                block.is_some_and(|block| block
                    .properties
                    .get_compound("vanilla_block_data")
                    .is_some()),
                "{name}"
            );
        }
    }
}
