mod data;

use std::collections::{BinaryHeap, HashMap, VecDeque};

use rand::{thread_rng, Rng};
use tbp::randomizer::SevenBag;
use tbp::MaybeUnknown;

use self::data::{Board, Piece, PieceLocation, Rotation, Spin};

use super::BattleConfigRaw;

pub struct Game {
    board: Board,
    queue: VecDeque<Piece>,
    hold: Option<Piece>,
    bag: Vec<Piece>,
    combo: u32,
    back_to_back: bool,
    garbage_queue: VecDeque<Garbage>,
    garbage_hole: usize,
}

struct Garbage {
    add_time: u64,
    amount: u32,
}

impl Game {
    pub fn new() -> Game {
        Game {
            board: Default::default(),
            queue: Default::default(),
            hold: None,
            bag: BAG.to_vec(),
            combo: 0,
            back_to_back: false,
            garbage_queue: Default::default(),
            garbage_hole: thread_rng().gen_range(0..10),
        }
    }

    pub fn refill_queue(&mut self, size: u32, mut f: impl FnMut(Piece)) {
        while self.queue.len() < size as usize {
            let i = thread_rng().gen_range(0..self.bag.len());
            let p = self.bag.swap_remove(i);
            self.queue.push_back(p);
            f(p);
            if self.bag.is_empty() {
                self.bag.extend_from_slice(&BAG);
            }
        }
    }

    pub fn start_msg(&self) -> tbp::frontend_msg::Start {
        let mut msg = tbp::frontend_msg::Start::new(
            self.hold.map(Into::into).map(MaybeUnknown::Known),
            self.queue
                .iter()
                .copied()
                .map(Into::into)
                .map(MaybeUnknown::Known)
                .collect(),
            self.combo,
            self.back_to_back,
            self.board.to_tbp(),
        );
        msg.randomizer = SevenBag::new(self.bag.iter().copied().map(Into::into).collect()).into();
        msg
    }

    pub fn counter_garbage(&mut self, amount: &mut u32) {
        while let Some(add) = self.garbage_queue.front_mut() {
            if add.amount <= *amount {
                *amount -= add.amount;
                self.garbage_queue.pop_front();
            } else {
                add.amount -= *amount;
                *amount = 0;
                break;
            }
        }
    }

    pub fn queue_garbage(&mut self, amount: u32, add_time: u64) {
        self.garbage_queue.push_back(Garbage { amount, add_time });
    }

    pub(super) fn add_garbage(&mut self, now: u64, config: &BattleConfigRaw) -> Vec<usize> {
        let mut added = vec![];
        while let Some(add) = self.garbage_queue.front() {
            if add.add_time > now {
                break;
            }
            for i in 0..add.amount {
                if i == 0 && config.garbage.change_on_attack
                    || thread_rng().gen_bool(config.garbage.messiness)
                {
                    let hole = thread_rng().gen_range(0..9);
                    if hole == self.garbage_hole {
                        self.garbage_hole = 9;
                    } else {
                        self.garbage_hole = hole;
                    }
                }
                added.push(self.garbage_hole);
            }
            self.garbage_queue.pop_front();
        }
        self.board.add_garbage(&added);
        added
    }

    pub(super) fn play_suggestion(
        &mut self,
        suggested: Vec<tbp::data::Move>,
        config: &BattleConfigRaw,
    ) -> Option<PlayedMove> {
        let mut next_moves = None;
        let mut hold_moves = None;
        for mv in suggested {
            let loc = match PieceLocation::try_from(mv.location.clone()) {
                Ok(v) => v.canonical_form(),
                Err(_) => continue,
            };
            let spin = match Spin::try_from(mv.spin.clone()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let next = self.queue[0];
            let hold = self.hold.unwrap_or(self.queue[1]);
            let group = if loc.piece == next {
                &mut next_moves
            } else if loc.piece == hold {
                &mut hold_moves
            } else {
                continue;
            };
            let group = group.get_or_insert_with(|| {
                self.movegen(loc.piece, config.delays.movement, config.delays.softdrop)
            });
            if let Some(&placement_delay) = group.get(&(loc, spin)) {
                let cleared = self.board.place(loc);
                self.queue.pop_front();
                if loc.piece == hold {
                    if self.hold.is_none() {
                        self.queue.pop_front();
                    }
                    self.hold = Some(next);
                }
                let mut clear_delay = 0;
                let mut garbage_sent = 0;
                if cleared == 0 {
                    self.combo = 0;
                } else {
                    let is_hard = spin != Spin::None || cleared == 4;

                    if self.board.is_pc() {
                        clear_delay += config.delays.pc[cleared - 1];
                    } else {
                        clear_delay += config.delays.clear[cleared - 1];
                    }

                    garbage_sent += match spin {
                        Spin::None => config.garbage.clear[cleared - 1],
                        Spin::Mini => config.garbage.mini[cleared - 1],
                        Spin::Full => config.garbage.spin[cleared - 1],
                    };
                    if self.back_to_back && is_hard {
                        garbage_sent += config.garbage.back_to_back;
                    }
                    garbage_sent += config.garbage.combo
                        [(self.combo as usize).min(config.garbage.combo.len() - 1)];

                    if self.board.is_pc() {
                        if config.garbage.pc_additive {
                            garbage_sent += config.garbage.pc[cleared - 1];
                        } else {
                            garbage_sent = config.garbage.pc[cleared - 1];
                        }
                    }

                    self.back_to_back = is_hard;
                    self.combo += 1;
                }

                return Some(PlayedMove {
                    mv,
                    clear: cleared > 0,
                    placement_delay,
                    clear_delay,
                    garbage_sent,
                });
            }
        }
        None
    }

    fn movegen(
        &self,
        piece: Piece,
        movement_delay: u32,
        softdrop_delay: u32,
    ) -> HashMap<(PieceLocation, Spin), u32> {
        let mut reached = vec![
            Cost {
                base: u32::MAX,
                softdrop: 0,
            };
            4800
        ];

        fn index(loc: PieceLocation, spin: Spin) -> usize {
            (loc.rotation as i32 + 4 * loc.x + 40 * spin as i32 + 120 * loc.y) as usize
        }

        let mut queue = BinaryHeap::new();
        let mut start = PieceLocation {
            x: 4,
            y: 19,
            rotation: Rotation::North,
            piece,
        };
        if start.obstructed(&self.board) {
            start.y += 1;
            if start.obstructed(&self.board) {
                return HashMap::new();
            }
        }
        reached[index(start, Spin::None)] = Cost {
            base: 0,
            softdrop: 0,
        };
        queue.push(QueueMove {
            loc: start,
            spin: Spin::None,
            cost: Cost {
                base: 0,
                softdrop: 0,
            },
        });

        let mut moves = HashMap::new();
        while let Some(mv) = queue.pop() {
            if reached[index(mv.loc, mv.spin)] != mv.cost {
                continue;
            }
            let mut reach = |mv: QueueMove| {
                let index = index(mv.loc, mv.spin);
                if mv.cost > reached[index] {
                    reached[index] = mv.cost;
                    queue.push(mv);
                }
            };
            // move left
            let loc = PieceLocation {
                x: mv.loc.x - 1,
                ..mv.loc
            };
            if !loc.obstructed(&self.board) {
                reach(QueueMove {
                    loc,
                    spin: Spin::None,
                    cost: Cost {
                        base: mv.cost.base + mv.cost.softdrop + movement_delay,
                        softdrop: 0,
                    },
                });
            }

            // move right
            let loc = PieceLocation {
                x: mv.loc.x + 1,
                ..mv.loc
            };
            if !loc.obstructed(&self.board) {
                reach(QueueMove {
                    loc,
                    spin: Spin::None,
                    cost: Cost {
                        base: mv.cost.base + mv.cost.softdrop + movement_delay,
                        softdrop: 0,
                    },
                });
            }

            // rotate cw
            for (i, loc) in mv.loc.rotate(mv.loc.rotation.cw()).enumerate() {
                if loc.obstructed(&self.board) {
                    continue;
                }
                let spin = check_spin(&self.board, loc, i);
                reach(QueueMove {
                    loc,
                    spin,
                    cost: Cost {
                        base: mv.cost.base + mv.cost.softdrop + movement_delay,
                        softdrop: 0,
                    },
                });
                break;
            }

            // rotate ccw
            for (i, loc) in mv.loc.rotate(mv.loc.rotation.ccw()).enumerate() {
                if loc.obstructed(&self.board) {
                    continue;
                }
                let spin = check_spin(&self.board, loc, i);
                reach(QueueMove {
                    loc,
                    spin,
                    cost: Cost {
                        base: mv.cost.base + mv.cost.softdrop + movement_delay,
                        softdrop: 0,
                    },
                });
                break;
            }

            // move down
            let loc = PieceLocation {
                y: mv.loc.y - 1,
                ..mv.loc
            };
            if loc.obstructed(&self.board) {
                let mvcost = moves
                    .entry((mv.loc.canonical_form(), mv.spin))
                    .or_insert(u32::MAX);
                *mvcost = mv.cost.base.min(*mvcost);
            } else {
                reach(QueueMove {
                    loc,
                    spin: Spin::None,
                    cost: Cost {
                        base: mv.cost.base,
                        softdrop: mv.cost.softdrop + softdrop_delay,
                    },
                });
            }
        }
        moves
    }
}

fn check_spin(board: &Board, loc: PieceLocation, kick: usize) -> Spin {
    if loc.piece != Piece::T {
        return Spin::None;
    }

    let mut mini_corners = 0;
    let mut norm_corners = 0;
    let (dx, dy) = loc.rotation.rotate(-1, 1);
    if board.get(dx + loc.x, dy + loc.y) {
        mini_corners += 1;
    }
    let (dx, dy) = loc.rotation.rotate(1, 1);
    if board.get(dx + loc.x, dy + loc.y) {
        mini_corners += 1;
    }
    let (dx, dy) = loc.rotation.rotate(-1, -1);
    if board.get(dx + loc.x, dy + loc.y) {
        norm_corners += 1;
    }
    let (dx, dy) = loc.rotation.rotate(1, -1);
    if board.get(dx + loc.x, dy + loc.y) {
        norm_corners += 1;
    }

    if norm_corners + mini_corners < 3 {
        Spin::None
    } else if mini_corners < 2 && kick != 4 {
        Spin::Mini
    } else {
        Spin::Full
    }
}

struct QueueMove {
    loc: PieceLocation,
    spin: Spin,
    cost: Cost,
}

#[derive(Copy, Clone, Debug)]
struct Cost {
    base: u32,
    softdrop: u32,
}

impl Ord for Cost {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.base
            .cmp(&other.base)
            .then(self.softdrop.cmp(&other.softdrop))
            .reverse()
    }
}

impl PartialOrd for Cost {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for Cost {}

impl PartialEq for Cost {
    fn eq(&self, other: &Self) -> bool {
        self.base == other.base && self.softdrop == other.softdrop
    }
}

impl Ord for QueueMove {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cost.cmp(&other.cost)
    }
}

impl PartialOrd for QueueMove {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for QueueMove {}

impl PartialEq for QueueMove {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

pub struct PlayedMove {
    pub mv: tbp::data::Move,
    pub clear: bool,
    pub placement_delay: u32,
    pub clear_delay: u32,
    pub garbage_sent: u32,
}

const BAG: [Piece; 7] = [
    Piece::I,
    Piece::O,
    Piece::T,
    Piece::L,
    Piece::J,
    Piece::S,
    Piece::Z,
];
