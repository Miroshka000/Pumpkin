#[allow(clippy::wildcard_imports)]
use super::*;

impl BedrockClient {
    /// Puts back a block the client already removed or replaced in its own prediction.
    pub(super) fn sync_block(&self, world: &crate::world::World, position: BlockPos) {
        let state = world.get_block_state(&position);
        let runtime_id = pumpkin_data::BlockState::to_be_network_id(state.id);
        self.try_enqueue_client_packet(&CUpdateBlock::new(position, runtime_id));
    }

    /// Fires the events the Java handler fires for a click on a block. Returns whether the click goes on.
    ///
    /// One press can bring several actions for one block, so a refusal holds until the next press.
    /// Otherwise a plugin that cancels the click would see it twice and the block would still break.
    fn allow_block_click(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        world: &crate::world::World,
        action: PlayerAction,
        location: BlockPos,
    ) -> bool {
        if matches!(action, PlayerAction::StartDestroyBlock) {
            self.denied_block.store(None);
        }
        if self.denied_block.load() == Some(location) {
            self.sync_block(world, location);
            return false;
        }
        let already_mining = player.mining.load(Ordering::Relaxed)
            && *player
                .mining_pos
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                == location;
        let (block, state) = world.get_block_and_state(&location);
        if already_mining || state.is_air() {
            return true;
        }

        let mut interact_event = PlayerInteractEvent::new(
            player,
            InteractAction::LeftClickBlock,
            block,
            Some(location),
        );
        server
            .plugin_manager
            .fire_blocking(server, &mut interact_event);
        let mut allowed = !interact_event.cancelled;
        if allowed {
            let mut damage_event =
                crate::plugin::api::events::block::block_damage::BlockDamageEvent::new(
                    player.clone(),
                    block,
                    location,
                    false,
                );
            server
                .plugin_manager
                .fire_blocking(server, &mut damage_event);
            allowed = !damage_event.cancelled;
        }
        if !allowed {
            player.stop_mining();
            self.denied_block.store(Some(location));
            self.sync_block(world, location);
        }
        allowed
    }

    #[expect(clippy::match_same_arms)]
    #[expect(clippy::too_many_lines)]
    pub fn handle_player_action(
        &self,
        player: &Arc<Player>,
        server: &Arc<Server>,
        packet: &SPlayerAction,
    ) {
        if !player.has_client_loaded()
            || ((player.living_entity.dead.load(Ordering::Relaxed)
                || player.living_entity.health.load() <= 0.0)
                && !matches!(packet.action, PlayerAction::Respawn))
        {
            return;
        }
        player.update_last_action_time();

        match packet.action {
            PlayerAction::StartDestroyBlock
            | PlayerAction::CreativeDestroyBlock
            | PlayerAction::ContinueDestroyBlock => {
                let location = packet.block_position;
                if !player.can_interact_with_block_at(&location, 1.0) {
                    return;
                }

                let entity = &player.get_entity();
                let world = entity.world.load_full();
                let (block, state) = world.get_block_and_state(&location);

                if player.mining.load(Ordering::Relaxed)
                    && *player
                        .mining_pos
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        != location
                {
                    player.stop_mining();
                }

                if !self.allow_block_click(player, server, &world, packet.action, location) {
                    return;
                }

                if player.gamemode.load() == GameMode::Creative {
                    let new_state = world.break_block(
                        &location,
                        Some(player),
                        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
                    );
                    if new_state.is_some() {
                        server
                            .block_registry
                            .broken(&world, block, player, &location, server, state);
                    } else {
                        self.sync_block(&world, location);
                    }
                } else if !state.is_air() {
                    let starts_breaking = !player.mining.load(Ordering::Relaxed)
                        || *player
                            .mining_pos
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            != location;
                    if starts_breaking {
                        server
                            .block_registry
                            .attack(&world, block, state, &location, player);
                    }
                    let speed = crate::block::calc_block_breaking(player, state, block);
                    if speed >= 1.0 {
                        player.stop_mining();
                        let broken_state = world.get_block_state(&location);
                        let can_harvest = player.can_harvest(broken_state, block);
                        let flags = if can_harvest {
                            BlockFlags::NOTIFY_ALL
                        } else {
                            BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL
                        };
                        let new_state = world.break_block(&location, Some(player), flags);
                        if new_state.is_some() {
                            server.block_registry.broken(
                                &world,
                                block,
                                player,
                                &location,
                                server,
                                broken_state,
                            );
                            player.apply_tool_damage_for_block_break(broken_state);
                            if can_harvest {
                                player.add_exhaustion(MINE_BLOCK_EXHAUSTION);
                            }
                            let item_id = player.inventory().held_item().item.id;
                            player.increment_stat(
                                pumpkin_data::statistic::StatisticCategory::Used,
                                item_id as i32,
                                1,
                            );
                            player.increment_stat(
                                pumpkin_data::statistic::StatisticCategory::Mined,
                                block.id.as_u16() as i32,
                                1,
                            );
                        } else {
                            self.sync_block(&world, location);
                        }
                    } else {
                        let mut mining_pos = player
                            .mining_pos
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let progress = if starts_breaking {
                            player.start_mining_time.store(
                                player.tick_counter.load(Ordering::Relaxed),
                                Ordering::Relaxed,
                            );
                            player.mining.store(true, Ordering::Relaxed);
                            *mining_pos = location;
                            (speed * 10.0) as i32
                        } else {
                            player.current_block_destroy_stage.load(Ordering::Relaxed)
                        };
                        drop(mining_pos);
                        let old_speed = player
                            .current_block_breaking_speed
                            .swap(speed.to_bits(), Ordering::Relaxed);
                        if starts_breaking {
                            world.set_block_breaking(
                                entity,
                                location,
                                BlockBreakingProgress::Start {
                                    stage: progress,
                                    speed,
                                },
                            );
                            player
                                .current_block_destroy_stage
                                .store(progress, Ordering::Relaxed);
                        } else if old_speed != speed.to_bits() {
                            world.set_block_breaking(
                                entity,
                                location,
                                BlockBreakingProgress::Update {
                                    stage: progress,
                                    speed: Some(speed),
                                },
                            );
                        }
                    }
                }
            }
            action @ (PlayerAction::PredictDestroyBlock | PlayerAction::StopDestroyBlock) => {
                let location = packet.block_position;
                if !player.can_interact_with_block_at(&location, 1.0) {
                    return;
                }

                let entity = &player.get_entity();
                let world = entity.world.load_full();

                let (block, state) = world.get_block_and_state(&location);
                if player.gamemode.load() != GameMode::Creative && !state.is_air() {
                    let speed = crate::block::calc_block_breaking(player, state, block);
                    let elapsed = player.tick_counter.load(Ordering::Relaxed)
                        - player.start_mining_time.load(Ordering::Relaxed)
                        + 1;
                    let same_block = *player
                        .mining_pos
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        == location;
                    if player.mining.load(Ordering::Relaxed)
                        && same_block
                        && speed * elapsed as f32 >= MIN_PREDICTED_BREAK_PROGRESS
                    {
                        player.stop_mining();

                        let can_harvest = player.can_harvest(state, block);
                        let flags = if can_harvest {
                            BlockFlags::NOTIFY_ALL
                        } else {
                            BlockFlags::SKIP_DROPS | BlockFlags::NOTIFY_ALL
                        };
                        if world.break_block(&location, Some(player), flags).is_some() {
                            server
                                .block_registry
                                .broken(&world, block, player, &location, server, state);
                            player.apply_tool_damage_for_block_break(state);
                            if can_harvest {
                                player.add_exhaustion(MINE_BLOCK_EXHAUSTION);
                            }
                            let item_id = player.inventory().held_item().item.id;
                            player.increment_stat(
                                pumpkin_data::statistic::StatisticCategory::Used,
                                item_id as i32,
                                1,
                            );
                            player.increment_stat(
                                pumpkin_data::statistic::StatisticCategory::Mined,
                                block.id.as_u16() as i32,
                                1,
                            );
                        } else {
                            self.sync_block(&world, location);
                        }
                    } else {
                        let runtime_id = pumpkin_data::BlockState::to_be_network_id(state.id);
                        self.try_enqueue_client_packet(&CUpdateBlock::new(location, runtime_id));
                        if matches!(action, PlayerAction::StopDestroyBlock) {
                            player.stop_mining();
                        } else {
                            world.set_block_breaking(
                                entity,
                                location,
                                BlockBreakingProgress::Update {
                                    stage: player
                                        .current_block_destroy_stage
                                        .load(Ordering::Relaxed),
                                    speed: Some(speed),
                                },
                            );
                        }
                    }
                } else if matches!(action, PlayerAction::StopDestroyBlock) {
                    player.stop_mining();
                }
            }
            PlayerAction::CrackBlock => {
                // Don't do anything for this action. It is no longer used. Block
                // cracking is done fully server-side.
            }
            PlayerAction::AbortDestroyBlock => {
                player.stop_mining();
            }
            PlayerAction::DropItem => {
                player.drop_held_item(false);
            }
            PlayerAction::StartGliding => {
                player.try_start_fall_flying(server);
            }
            PlayerAction::StopGliding => {
                player.stop_fall_flying(server);
            }
            PlayerAction::StartSwimming => {
                player.get_entity().set_swimming(true);
                player.update_player_pose();
            }
            PlayerAction::StopSwimming => {
                player.get_entity().set_swimming(false);
                player.update_player_pose();
            }
            PlayerAction::StartSpinAttack => {
                player
                    .get_entity()
                    .set_pose(pumpkin_data::entity::EntityPose::SpinAttack);
            }
            PlayerAction::StopSpinAttack => {
                player.update_player_pose();
            }
            PlayerAction::StartSprinting => {
                player.set_sprinting(true);
            }
            PlayerAction::StopSprinting => {
                player.set_sprinting(false);
            }
            PlayerAction::StartSneaking => {
                player.get_entity().set_sneaking(true);
                player.update_bedrock_shield_sneaking();
            }
            PlayerAction::StopSneaking => {
                player.get_entity().set_sneaking(false);
                player.update_bedrock_shield_sneaking();
            }
            PlayerAction::StartCrawling => {
                player
                    .get_entity()
                    .set_pose(pumpkin_data::entity::EntityPose::Swimming);
            }
            PlayerAction::StopCrawling => {
                player.update_player_pose();
            }
            PlayerAction::StopSleeping => {
                player.wake_up();
            }
            PlayerAction::Respawn
                if player.living_entity.dead.load(Ordering::Relaxed)
                    || player.living_entity.health.load() <= 0.0 =>
            {
                let player_c = player.clone();
                player.spawn_task(async move {
                    player_c.world().respawn_player(&player_c, false).await;
                });
            }
            // TODO
            _ => {}
        }
    }
}
