use tbp::MaybeUnknown;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Piece {
    I,
    O,
    T,
    L,
    J,
    S,
    Z,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Rotation {
    North,
    East,
    South,
    West,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Spin {
    None,
    Mini,
    Full,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PieceLocation {
    pub piece: Piece,
    pub rotation: Rotation,
    pub x: i32,
    pub y: i32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Board {
    field: [[CellColor; 10]; 40],
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CellColor {
    Piece(Piece),
    Garbage,
    Empty,
}

impl Piece {
    fn cells(self) -> [(i32, i32); 4] {
        match self {
            Piece::I => [(-1, 0), (0, 0), (1, 0), (2, 0)],
            Piece::O => [(0, 0), (1, 0), (0, 1), (1, 1)],
            Piece::T => [(-1, 0), (0, 0), (1, 0), (0, 1)],
            Piece::L => [(-1, 0), (0, 0), (1, 0), (1, 1)],
            Piece::J => [(-1, 0), (0, 0), (1, 0), (-1, 1)],
            Piece::S => [(-1, 0), (0, 0), (0, 1), (1, 1)],
            Piece::Z => [(-1, 1), (0, 1), (0, 0), (1, 0)],
        }
    }
}

impl Rotation {
    pub fn rotate(self, x: i32, y: i32) -> (i32, i32) {
        match self {
            Rotation::North => (x, y),
            Rotation::East => (y, -x),
            Rotation::South => (-x, -y),
            Rotation::West => (-y, x),
        }
    }

    pub fn cw(self) -> Self {
        match self {
            Rotation::North => Rotation::East,
            Rotation::East => Rotation::South,
            Rotation::South => Rotation::West,
            Rotation::West => Rotation::North,
        }
    }

    pub fn ccw(self) -> Self {
        match self {
            Rotation::North => Rotation::West,
            Rotation::East => Rotation::North,
            Rotation::South => Rotation::East,
            Rotation::West => Rotation::South,
        }
    }
}

impl PieceLocation {
    fn cells(self) -> [(i32, i32); 4] {
        self.piece
            .cells()
            .map(|(x, y)| self.rotation.rotate(x, y))
            .map(|(x, y)| (x + self.x, y + self.y))
    }

    pub fn obstructed(self, board: &Board) -> bool {
        for (x, y) in self.cells() {
            if board.get(x, y) {
                return true;
            }
        }
        false
    }

    pub fn rotate(self, rot: Rotation) -> impl Iterator<Item = PieceLocation> {
        offsets(self.piece, self.rotation)
            .zip(offsets(self.piece, rot))
            .map(move |((x1, y1), (x2, y2))| PieceLocation {
                x: self.x + x1 - x2,
                y: self.y + y1 - y2,
                rotation: rot,
                piece: self.piece,
            })
    }

    pub fn canonical_form(self) -> PieceLocation {
        match self.piece {
            Piece::T | Piece::J | Piece::L => self,
            Piece::O => match self.rotation {
                Rotation::North => self,
                Rotation::East => PieceLocation {
                    rotation: Rotation::North,
                    y: self.y - 1,
                    ..self
                },
                Rotation::South => PieceLocation {
                    rotation: Rotation::North,
                    x: self.x - 1,
                    y: self.y - 1,
                    ..self
                },
                Rotation::West => PieceLocation {
                    rotation: Rotation::North,
                    x: self.x - 1,
                    ..self
                },
            },
            Piece::S | Piece::Z => match self.rotation {
                Rotation::North | Rotation::East => self,
                Rotation::South => PieceLocation {
                    rotation: Rotation::North,
                    y: self.y - 1,
                    ..self
                },
                Rotation::West => PieceLocation {
                    rotation: Rotation::East,
                    x: self.x - 1,
                    ..self
                },
            },
            Piece::I => match self.rotation {
                Rotation::North | Rotation::East => self,
                Rotation::South => PieceLocation {
                    rotation: Rotation::North,
                    x: self.x - 1,
                    ..self
                },
                Rotation::West => PieceLocation {
                    rotation: Rotation::East,
                    y: self.y + 1,
                    ..self
                },
            },
        }
    }
}

impl Board {
    pub fn place(&mut self, piece: PieceLocation) -> usize {
        for (x, y) in piece.cells() {
            self.field[y as usize][x as usize] = CellColor::Piece(piece.piece);
        }
        let mut row = 0;
        for i in 0..40 {
            if self.field[i].iter().all(|&c| c != CellColor::Empty) {
                continue;
            }
            self.field[row] = self.field[i];
            row += 1;
        }
        for i in row..40 {
            self.field[i] = [CellColor::Empty; 10];
        }
        40 - row
    }

    pub fn get(&self, x: i32, y: i32) -> bool {
        self.field
            .get(y as usize)
            .and_then(|a| a.get(x as usize))
            .copied()
            != Some(CellColor::Empty)
    }

    pub fn is_pc(&self) -> bool {
        self.field[0] == [CellColor::Empty; 10]
    }

    pub fn add_garbage(&mut self, cols: &[usize]) {
        for y in (0..40).rev() {
            if y < cols.len() {
                let i = cols.len() - y - 1;
                self.field[y] = [CellColor::Garbage; 10];
                self.field[y][cols[i]] = CellColor::Empty;
            } else {
                self.field[y] = self.field[y - cols.len()];
            }
        }
    }

    pub fn height(&self) -> i32 {
        self.field.partition_point(|r| r != &[CellColor::Empty; 10]) as i32
    }

    pub fn to_tbp(&self) -> Vec<Vec<Option<char>>> {
        let mut result = Vec::with_capacity(40);
        for r in self.field {
            let mut row = Vec::with_capacity(10);
            for c in r {
                match c {
                    CellColor::Piece(Piece::I) => row.push(Some('I')),
                    CellColor::Piece(Piece::O) => row.push(Some('O')),
                    CellColor::Piece(Piece::T) => row.push(Some('T')),
                    CellColor::Piece(Piece::L) => row.push(Some('L')),
                    CellColor::Piece(Piece::J) => row.push(Some('J')),
                    CellColor::Piece(Piece::S) => row.push(Some('S')),
                    CellColor::Piece(Piece::Z) => row.push(Some('Z')),
                    CellColor::Garbage => row.push(Some('G')),
                    CellColor::Empty => row.push(None),
                }
            }
            result.push(row);
        }
        result
    }
}

fn offsets(piece: Piece, rotation: Rotation) -> impl Iterator<Item = (i32, i32)> {
    match piece {
        Piece::O => match rotation {
            Rotation::North => &[(0, 0)],
            Rotation::East => &[(0, -1)],
            Rotation::South => &[(-1, -1)],
            Rotation::West => &[(-1, 0)],
        }
        .iter()
        .copied(),
        Piece::I => match rotation {
            Rotation::North => &[(0, 0), (-1, 0), (2, 0), (-1, 0), (2, 0)],
            Rotation::East => &[(-1, 0), (0, 0), (0, 0), (0, 1), (0, -2)],
            Rotation::South => &[(-1, 1), (1, 1), (-2, 1), (1, 0), (-2, 0)],
            Rotation::West => &[(0, 1), (0, 1), (0, 1), (0, -1), (0, 2)],
        }
        .iter()
        .copied(),
        _ => match rotation {
            Rotation::North => &[(0, 0); 5],
            Rotation::East => &[(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
            Rotation::South => &[(0, 0); 5],
            Rotation::West => &[(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
        }
        .iter()
        .copied(),
    }
}

impl Default for Board {
    fn default() -> Self {
        Self {
            field: [[CellColor::Empty; 10]; 40],
        }
    }
}

impl TryFrom<MaybeUnknown<tbp::data::Piece>> for Piece {
    type Error = anyhow::Error;

    fn try_from(value: MaybeUnknown<tbp::data::Piece>) -> anyhow::Result<Self> {
        use tbp::data::Piece;
        Ok(match value {
            MaybeUnknown::Known(Piece::I) => Self::I,
            MaybeUnknown::Known(Piece::O) => Self::O,
            MaybeUnknown::Known(Piece::T) => Self::T,
            MaybeUnknown::Known(Piece::L) => Self::L,
            MaybeUnknown::Known(Piece::J) => Self::J,
            MaybeUnknown::Known(Piece::S) => Self::S,
            MaybeUnknown::Known(Piece::Z) => Self::Z,
            _ => anyhow::bail!("invalid piece"),
        })
    }
}

impl TryFrom<MaybeUnknown<tbp::data::Orientation>> for Rotation {
    type Error = anyhow::Error;

    fn try_from(value: MaybeUnknown<tbp::data::Orientation>) -> anyhow::Result<Self> {
        use tbp::data::Orientation;
        Ok(match value {
            MaybeUnknown::Known(Orientation::North) => Self::North,
            MaybeUnknown::Known(Orientation::East) => Self::East,
            MaybeUnknown::Known(Orientation::South) => Self::South,
            MaybeUnknown::Known(Orientation::West) => Self::West,
            _ => anyhow::bail!("invalid orientation"),
        })
    }
}

impl From<Piece> for tbp::data::Piece {
    fn from(value: Piece) -> Self {
        match value {
            Piece::I => Self::I,
            Piece::O => Self::O,
            Piece::T => Self::T,
            Piece::L => Self::L,
            Piece::J => Self::J,
            Piece::S => Self::S,
            Piece::Z => Self::Z,
        }
    }
}

impl From<Rotation> for tbp::data::Orientation {
    fn from(value: Rotation) -> Self {
        match value {
            Rotation::North => Self::North,
            Rotation::East => Self::East,
            Rotation::South => Self::South,
            Rotation::West => Self::West,
        }
    }
}

impl TryFrom<tbp::data::PieceLocation> for PieceLocation {
    type Error = anyhow::Error;

    fn try_from(value: tbp::data::PieceLocation) -> anyhow::Result<Self> {
        Ok(PieceLocation {
            piece: value.kind.try_into()?,
            rotation: value.orientation.try_into()?,
            x: value.x,
            y: value.y,
        })
    }
}

impl TryFrom<MaybeUnknown<tbp::data::Spin>> for Spin {
    type Error = anyhow::Error;

    fn try_from(value: MaybeUnknown<tbp::data::Spin>) -> anyhow::Result<Self> {
        use tbp::data::Spin;
        Ok(match value {
            MaybeUnknown::Known(Spin::None) => Self::None,
            MaybeUnknown::Known(Spin::Mini) => Self::Mini,
            MaybeUnknown::Known(Spin::Full) => Self::Full,
            _ => anyhow::bail!("invalid spin"),
        })
    }
}
