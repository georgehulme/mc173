//! Lake generation.

use glam::{IVec3, DVec3};

use crate::util::JavaRandom;
use crate::world::World;
use crate::block;

use super::FeatureGenerator;


pub struct LakeGenerator {
    fluid_id: u8,
}

impl LakeGenerator {

    /// Create a new lake generator for the given block id.
    pub fn new(fluid_id: u8) -> Self {
        Self { fluid_id, }
    }

}

impl FeatureGenerator for LakeGenerator {

    fn generate(&mut self, world: &mut World, mut pos: IVec3, rand: &mut JavaRandom) -> bool {

        // Lake have a maximum size of 16x8x16, so we subtract half.
        pos -= IVec3::new(8, 0, 8);
        while pos.y > 0 && world.is_block_air(pos) {
            pos.y -= 1;
        }
        pos.y -= 4;

        // [X][Z][Y]
        let mut fill = Box::new([[[false; 8]; 16]; 16]);

        let count = rand.next_int_bounded(4) + 4;
        for _ in 0..count {

            let a = rand.next_dvec3() * 
                DVec3::new(6.0, 4.0, 6.0) + 
                DVec3::new(3.0, 2.0, 3.0);

            let b = rand.next_dvec3() * 
                (DVec3::new(16.0, 8.0, 16.0) - a - DVec3::new(2.0, 4.0, 2.0)) +
                DVec3::new(1.0, 2.0, 1.0) + a / 2.0;

            let a = a / 2.0;

            for dx in 1..15 {
                for dz in 1..15 {
                    for dy in 1..7 {
                        let dist = (DVec3::new(dx as f64, dy as f64, dz as f64) - b) / a;
                        if dist.length_squared() < 1.0 {
                            fill[dx][dz][dy] = true;
                        }
                    }
                }
            }

        }

        for dx in 0..16 {
            for dz in 0..16 {
                for dy in 0..8 {

                    let filled = !fill[dx][dz][dy] && (
                        dx < 15 && fill[dx + 1][dz][dy] ||
                        dx > 0  && fill[dx - 1][dz][dy] ||
                        dz < 15 && fill[dx][dz + 1][dy] ||
                        dz > 0  && fill[dx][dz - 1][dy] ||
                        dy < 7  && fill[dx][dz][dy + 1] ||
                        dy > 0  && fill[dx][dz][dy - 1]
                    );

                    if filled {
                        let check_pos = pos + IVec3::new(dx as i32, dy as i32, dz as i32);
                        let check_id = world.get_block(check_pos).map(|(id, _)| id).unwrap_or(block::AIR);
                        let check_material = block::from_id(check_id).material;
                        if dy >= 4 && check_material.is_fluid() {
                            return false;
                        } else if dy < 4 && !check_material.is_solid() && check_id != self.fluid_id {
                            return false;
                        }
                    }

                }
            }
        }

        for dx in 0..16 {
            for dz in 0..16 {
                for dy in 0..8 {
                    if fill[dx][dz][dy] {
                        let place_pos = pos + IVec3::new(dx as i32, dy as i32, dz as i32);
                        world.set_block(place_pos, if dy >= 4 { block::AIR } else { self.fluid_id }, 0);
                    }
                }
            }
        }

        for dx in 0..16 {
            for dz in 0..16 {
                for dy in 4..8 {
                    if fill[dx][dz][dy] {
                        let check_pos = pos + IVec3::new(dx as i32, dy as i32 - 1, dz as i32);
                        if world.is_block(check_pos, block::DIRT) {
                            if let Some(light) = world.get_light(check_pos + IVec3::Y, false) {
                                if light.sky > 0 {
                                    world.set_block(check_pos, block::GRASS, 0);
                                }
                            }
                        }
                    }
                }
            }
        }

        if let block::LAVA_STILL | block::LAVA_MOVING = self.fluid_id {
            for dx in 0..16 {
                for dz in 0..16 {
                    for dy in 0..8 {

                        let filled = !fill[dx][dz][dy] && (
                            dx < 15 && fill[dx + 1][dz][dy] ||
                            dx > 0  && fill[dx - 1][dz][dy] ||
                            dz < 15 && fill[dx][dz + 1][dy] ||
                            dz > 0  && fill[dx][dz - 1][dy] ||
                            dy < 7  && fill[dx][dz][dy + 1] ||
                            dy > 0  && fill[dx][dz][dy - 1]
                        );

                        if filled && (dy < 4 || rand.next_int_bounded(2) != 0) {
                            let place_pos = pos + IVec3::new(dx as i32, dy as i32, dz as i32);
                            if world.get_block_material(place_pos).is_solid() {
                                world.set_block(place_pos, block::STONE, 0);
                            }
                        }

                    }
                }
            }
        }

        true

    }

}
