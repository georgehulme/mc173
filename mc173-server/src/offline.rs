//! Offline player data.

use glam::{DVec3, Vec2};
use mc173::item::ItemStack;

/// An offline player defines the saved data of a player that is not connected.
#[derive(Debug)]
pub struct OfflinePlayer {
    /// World name.
    pub world: String,
    /// Last saved position of the player.
    pub pos: DVec3,
    /// Last saved look of the player.
    pub look: Vec2,
    /// The main player inventory including the hotbar in the first 9 slots.
    pub main_inv: Box<[ItemStack; 36]>,
    /// The armor player inventory.
    pub armor_inv: Box<[ItemStack; 4]>,
    /// The item stacks for the 3x3 crafting grid. Also support the 2x2 as top left slots.
    pub craft_inv: Box<[ItemStack; 9]>,
    /// The item stack in the cursor of the client's using a window.
    pub cursor_stack: ItemStack,
    /// The slot current selected for the hand. Must be in range 0..9.
    pub hand_slot: u8,
}

impl OfflinePlayer {
    pub fn new(world: String, pos: DVec3) -> Self {
        Self {
            world,
            pos,
            look: Vec2::ZERO,
            main_inv: Box::new([ItemStack::EMPTY; 36]),
            armor_inv: Box::new([ItemStack::EMPTY; 4]),
            craft_inv: Box::new([ItemStack::EMPTY; 9]),
            cursor_stack: ItemStack::EMPTY,
            hand_slot: 0,
        }
    }
}
