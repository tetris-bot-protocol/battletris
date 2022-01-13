use std::collections::BinaryHeap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::bot::BotInstance;

use self::game::Game;

mod game;

#[derive(Deserialize)]
struct BattleConfigRaw {
    time_quanta_ms: u64,
    next_queue_size: u32,
    delays: Delays,
    garbage: Garbage,
}

#[derive(Deserialize)]
struct Delays {
    start: u32,
    spawn: u32,
    movement: u32,
    softdrop: u32,
    clear: [u32; 4],
    pc: [u32; 4],
    garbage: u32,
}

#[derive(Deserialize)]
struct Garbage {
    clear: [u32; 4],
    mini: [u32; 3],
    spin: [u32; 3],
    back_to_back: u32,
    pc: [u32; 4],
    pc_additive: bool,
    combo: Vec<u32>,
    change_on_attack: bool,
    messiness: f64,
    countering: bool,
    blocking: bool,
}

#[derive(Deserialize)]
#[serde(try_from = "BattleConfigRaw")]
pub struct BattleConfig(BattleConfigRaw);

#[derive(Copy, Clone, Debug)]
pub enum Side {
    Left,
    Right,
}

pub fn battle(
    left: &mut BotInstance,
    right: &mut BotInstance,
    BattleConfig(config): &BattleConfig,
    running: &AtomicBool,
) -> Option<Side> {
    let mut event_queue = BinaryHeap::new();
    event_queue.push(Event {
        side: Side::Left,
        time: config.delays.start as u64,
        event: EventType::RequestMove,
    });
    event_queue.push(Event {
        side: Side::Right,
        time: config.delays.start as u64,
        event: EventType::RequestMove,
    });

    let mut left_game = Game::new();
    let mut right_game = Game::new();
    left_game.refill_queue(config.next_queue_size, |_| {});
    right_game.refill_queue(config.next_queue_size, |_| {});

    let _ = left.send_message(left_game.start_msg());
    let _ = right.send_message(right_game.start_msg());

    let start_time = Instant::now();
    let winner = loop {
        let event = event_queue.pop().unwrap();
        let next_time = start_time + Duration::from_millis(config.time_quanta_ms * event.time);
        let now = Instant::now();
        if next_time > now {
            std::thread::sleep(next_time - now);
        }

        if !running.load(Ordering::SeqCst) {
            return None;
        }

        let current = start_time.elapsed().as_millis() as u64 / config.time_quanta_ms;

        let (bot, _opp_bot) = match event.side {
            Side::Left => (&mut *left, &mut *right),
            Side::Right => (&mut *right, &mut *left),
        };
        let (game, opp_game) = match event.side {
            Side::Left => (&mut left_game, &mut right_game),
            Side::Right => (&mut right_game, &mut left_game),
        };
        let opponent = match event.side {
            Side::Left => Side::Right,
            Side::Right =>Side::Left,
        };

        match event.event {
            EventType::RequestMove => {
                let _ = bot.send_message(tbp::frontend_msg::Suggest::new());
                event_queue.push(Event {
                    time: current + 1,
                    side: event.side,
                    event: EventType::PollMove(current),
                });
            }
            EventType::PollMove(requested) => {
                match bot.poll_message() {
                    Err(_) => break opponent,
                    Ok(None) => {
                        event_queue.push(Event {
                            time: current + 1,
                            ..event
                        });
                        if (current - requested) * config.time_quanta_ms > 500 {
                            break opponent;
                        }
                    }
                    Ok(Some(tbp::BotMessage::Suggestion(suggestion))) => {
                        let result = game.play_suggestion(suggestion.moves, config);
                        if let Some(played) = result {
                            let _ = bot.send_message(tbp::frontend_msg::Play::new(played.mv));
                            if played.clear && config.garbage.blocking {
                                event_queue.push(Event {
                                    side: event.side,
                                    time: current
                                        + (played.placement_delay
                                            + played.clear_delay
                                            + config.delays.spawn)
                                            as u64,
                                    event: EventType::RequestMove,
                                });
                            } else {
                                event_queue.push(Event {
                                    side: event.side,
                                    time: current
                                        + (played.placement_delay + played.clear_delay) as u64,
                                    event: EventType::CheckGarbage,
                                });
                            }
                            if played.garbage_sent > 0 {
                                event_queue.push(Event {
                                    side: event.side,
                                    time: current + played.placement_delay as u64,
                                    event: EventType::SendGarbage(played.garbage_sent),
                                });
                            }
                        } else {
                            break opponent;
                        }
                        game.refill_queue(config.next_queue_size, |p| {
                            let _ = bot.send_message(tbp::frontend_msg::NewPiece::new(
                                tbp::MaybeUnknown::Known(p.into()),
                            ));
                        });
                    }
                    Ok(_) => {}
                }
            }
            EventType::SendGarbage(mut amount) => {
                if config.garbage.countering {
                    game.counter_garbage(&mut amount);
                }
                if amount != 0 {
                    opp_game.queue_garbage(amount, current + config.delays.garbage as u64);
                }
            }
            EventType::CheckGarbage => {
                if !game.add_garbage(current, config).is_empty() {
                    let _ = bot.send_message(game.start_msg());
                }
                event_queue.push(Event {
                    side: event.side,
                    time: current + config.delays.spawn as u64,
                    event: EventType::RequestMove,
                });
            }
        }
    };

    while let Some(event) = event_queue.pop() {
        if let EventType::PollMove(_) = event.event {
            let _ = match event.side {
                Side::Left => left.block_message(),
                Side::Right => right.block_message(),
            };
        }
    }

    Some(winner)
}

#[derive(Copy, Clone, Debug)]
struct Event {
    side: Side,
    time: u64,
    event: EventType,
}

#[derive(Copy, Clone, Debug)]
enum EventType {
    PollMove(u64),
    RequestMove,
    CheckGarbage,
    SendGarbage(u32),
}

impl EventType {
    fn priority(&self) -> u32 {
        match self {
            EventType::PollMove(_) => 3,
            EventType::RequestMove => 2,
            EventType::CheckGarbage => 1,
            EventType::SendGarbage(_) => 0,
        }
    }
}

impl Ord for Event {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.time
            .cmp(&other.time)
            .then_with(|| self.event.priority().cmp(&other.event.priority()))
            .reverse()
    }
}

impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for Event {}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time
    }
}

impl TryFrom<BattleConfigRaw> for BattleConfig {
    type Error = anyhow::Error;

    fn try_from(value: BattleConfigRaw) -> anyhow::Result<Self> {
        if !(1..10000).contains(&value.time_quanta_ms) {
            anyhow::bail!("time_quanta_ms must be between 1 and 10000 milliseconds");
        }
        Ok(Self(value))
    }
}

impl FromStr for BattleConfig {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((left, mut right)) = s.split_once("@") {
            right = right.strip_suffix("ms").unwrap_or(right);
            let time_quanta = right.parse()?;
            let mut config = BattleConfigRaw::named_config(left)
                .ok_or_else(|| anyhow::anyhow!("Invalid battle config name `{}`", left))?;
            config.time_quanta_ms = time_quanta;
            Ok(config.try_into()?)
        } else if let Some(config) = BattleConfigRaw::named_config(s) {
            Ok(config.try_into()?)
        } else {
            Ok(serde_json::from_str(s)?)
        }
    }
}

impl BattleConfigRaw {
    fn named_config(s: &str) -> Option<BattleConfigRaw> {
        Some(match s {
            "ppt" => Self {
                time_quanta_ms: 16,
                delays: Delays {
                    start: 180,
                    spawn: 7,
                    movement: 2,
                    softdrop: 2,
                    clear: [36, 41, 41, 46],
                    pc: [1, 1, 1, 1],
                    garbage: 30,
                },
                garbage: Garbage {
                    clear: [0, 1, 2, 4],
                    mini: [0, 1, 2],
                    spin: [2, 4, 6],
                    back_to_back: 1,
                    pc: [10, 10, 10, 10],
                    pc_additive: false,
                    combo: vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 4, 5],
                    change_on_attack: true,
                    messiness: 0.3,
                    countering: true,
                    blocking: false,
                },
                next_queue_size: 5,
            },
            _ => return None,
        })
    }
}
