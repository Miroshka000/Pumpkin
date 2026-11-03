// Last verified for v2225

use pumpkin_macros::packet;
use pumpkin_nbt::compound::NbtCompound;

use crate::serial::PacketWrite;

/// Declares the client synced properties of one entity type.
///
/// The vanilla resource pack reads these with `query.property` for sounds and textures,
/// and a client only knows the ones a server declared after `StartGame`.
#[derive(PacketWrite)]
#[packet(165)]
pub struct CSyncActorProperty {
    /// Network NBT compound: `type` is the entity identifier, `properties` lists the definitions.
    pub property_data: NbtCompound,
}
