// Spec 3 — authoritative room membership.
//
// Shared, rendering- and network-free door model. Both the server (validation
// against `Door::server_trigger`) and the client (detection against the raw
// `Door::trigger`) derive "is the player at this door" from this data.

use crate::rect::Rect;

/// Room coordinates in the same convention as the server's `coords` and the
/// `PlayerPos` packet: `(level_x, level_y) = (column, row)`.
pub type RoomCoords = (u8, u8);

/// Which edge of a room a door sits on.
///
/// The variant order is part of the wire format: postcard encodes the variant
/// index. Do not reorder after release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    /// Variant index 0.
    North,
    /// Variant index 1.
    South,
    /// Variant index 2.
    East,
    /// Variant index 3.
    West,
}

/// A directed edge from one room edge to the adjacent room.
///
/// Bidirectional openings are represented by two doors (A→B East and B→A
/// West), each with its own trigger band and canonical entry position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Door {
    /// Source room.
    pub from: RoomCoords,
    /// Edge of `from` the door sits on.
    pub direction: Direction,
    /// Room entered when the door is used.
    pub target: RoomCoords,
    /// Geometric band used by the client's `door_at` detection.
    pub trigger: Rect,
    /// Canonical player top-left position inside `target` after the teleport.
    pub entry: (f32, f32),
}

impl Door {
    /// Server-side validation rect: the geometric `trigger` grown by
    /// `DOOR_LEAD_TOLERANCE_PX` on the approach edge (East: left, West: right,
    /// South: top, North: bottom), so a request issued on the predicted-first
    /// client tick still validates against the slightly-behind authoritative
    /// position. The client's `door_at` uses the raw `trigger`, never this.
    pub fn server_trigger(&self) -> Rect {
        let tol = crate::config::DOOR_LEAD_TOLERANCE_PX;
        let t = self.trigger;
        match self.direction {
            Direction::East => Rect::new(t.x - tol, t.y, t.w + tol, t.h),
            Direction::West => Rect::new(t.x, t.y, t.w + tol, t.h),
            Direction::South => Rect::new(t.x, t.y - tol, t.w, t.h + tol),
            Direction::North => Rect::new(t.x, t.y, t.w, t.h + tol),
        }
    }
}

/// The complete door graph of the current 2×2 map, in table order.
#[derive(Debug, Clone)]
pub struct RoomGraph {
    doors: Vec<Door>,
}

impl RoomGraph {
    /// Builds the eight directed doors of the current map.
    ///
    /// Table order is load-bearing for `door_at` tie-breaking, though the
    /// current map has no overlapping triggers.
    pub fn new() -> Self {
        Self {
            doors: vec![
                Door {
                    from: (0, 0),
                    direction: Direction::East,
                    target: (1, 0),
                    trigger: Rect::new(760.0, 320.0, 40.0, 40.0),
                    entry: (10.0, 321.0),
                },
                Door {
                    from: (1, 0),
                    direction: Direction::West,
                    target: (0, 0),
                    trigger: Rect::new(0.0, 320.0, 40.0, 40.0),
                    entry: (760.0, 321.0),
                },
                Door {
                    from: (0, 0),
                    direction: Direction::South,
                    target: (0, 1),
                    trigger: Rect::new(280.0, 440.0, 80.0, 40.0),
                    entry: (305.0, 10.0),
                },
                Door {
                    from: (0, 1),
                    direction: Direction::North,
                    target: (0, 0),
                    trigger: Rect::new(280.0, 0.0, 80.0, 40.0),
                    entry: (305.0, 432.0),
                },
                Door {
                    from: (1, 0),
                    direction: Direction::South,
                    target: (1, 1),
                    trigger: Rect::new(480.0, 440.0, 80.0, 40.0),
                    entry: (505.0, 10.0),
                },
                Door {
                    from: (1, 1),
                    direction: Direction::North,
                    target: (1, 0),
                    trigger: Rect::new(480.0, 0.0, 80.0, 40.0),
                    entry: (505.0, 432.0),
                },
                Door {
                    from: (0, 1),
                    direction: Direction::East,
                    target: (1, 1),
                    trigger: Rect::new(760.0, 400.0, 40.0, 40.0),
                    entry: (10.0, 401.0),
                },
                Door {
                    from: (1, 1),
                    direction: Direction::West,
                    target: (0, 1),
                    trigger: Rect::new(0.0, 400.0, 40.0, 40.0),
                    entry: (760.0, 401.0),
                },
            ],
        }
    }

    pub fn doors(&self) -> &[Door] {
        &self.doors
    }

    /// The unique door leaving `from` through `direction`, if the map has one.
    pub fn door(&self, from: RoomCoords, direction: Direction) -> Option<Door> {
        self.doors
            .iter()
            .find(|d| d.from == from && d.direction == direction)
            .copied()
    }

    /// Client-side detection: the first door whose **raw geometric** trigger
    /// overlaps `player_rect`. Never uses `server_trigger`.
    pub fn door_at(&self, from: RoomCoords, player_rect: Rect) -> Option<Door> {
        self.doors
            .iter()
            .find(|d| d.from == from && d.trigger.overlaps(&player_rect))
            .copied()
    }

    /// Server-side validation: a door exists for `(from, direction)` and the
    /// authoritative player rect overlaps its lead-expanded `server_trigger`.
    pub fn can_enter(&self, from: RoomCoords, direction: Direction, player_rect: Rect) -> bool {
        self.door(from, direction)
            .map(|d| d.server_trigger().overlaps(&player_rect))
            .unwrap_or(false)
    }
}

impl Default for RoomGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, Door, RoomGraph};
    use crate::level::Level;
    use crate::maps::Map;
    use crate::rect::Rect;
    use crate::types::TileType;

    fn level_for(map: &Map, room: (u8, u8)) -> Level {
        Level::new(
            map.grids[room.1 as usize][room.0 as usize].clone(),
            map.data[room.1 as usize][room.0 as usize].clone(),
        )
    }

    fn opposite(direction: Direction) -> Direction {
        match direction {
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::East => Direction::West,
            Direction::West => Direction::East,
        }
    }

    /// The spec's complete eight-door table, in table order.
    fn table() -> [Door; 8] {
        [
            Door {
                from: (0, 0),
                direction: Direction::East,
                target: (1, 0),
                trigger: Rect::new(760.0, 320.0, 40.0, 40.0),
                entry: (10.0, 321.0),
            },
            Door {
                from: (1, 0),
                direction: Direction::West,
                target: (0, 0),
                trigger: Rect::new(0.0, 320.0, 40.0, 40.0),
                entry: (760.0, 321.0),
            },
            Door {
                from: (0, 0),
                direction: Direction::South,
                target: (0, 1),
                trigger: Rect::new(280.0, 440.0, 80.0, 40.0),
                entry: (305.0, 10.0),
            },
            Door {
                from: (0, 1),
                direction: Direction::North,
                target: (0, 0),
                trigger: Rect::new(280.0, 0.0, 80.0, 40.0),
                entry: (305.0, 432.0),
            },
            Door {
                from: (1, 0),
                direction: Direction::South,
                target: (1, 1),
                trigger: Rect::new(480.0, 440.0, 80.0, 40.0),
                entry: (505.0, 10.0),
            },
            Door {
                from: (1, 1),
                direction: Direction::North,
                target: (1, 0),
                trigger: Rect::new(480.0, 0.0, 80.0, 40.0),
                entry: (505.0, 432.0),
            },
            Door {
                from: (0, 1),
                direction: Direction::East,
                target: (1, 1),
                trigger: Rect::new(760.0, 400.0, 40.0, 40.0),
                entry: (10.0, 401.0),
            },
            Door {
                from: (1, 1),
                direction: Direction::West,
                target: (0, 1),
                trigger: Rect::new(0.0, 400.0, 40.0, 40.0),
                entry: (760.0, 401.0),
            },
        ]
    }

    /// G1 — the graph has exactly the eight doors and every
    /// `(from, direction)` pair is unique.
    #[test]
    fn room_graph_has_eight_unique_doors() {
        let graph = RoomGraph::new();
        let doors = graph.doors();
        assert_eq!(doors.len(), 8);

        for i in 0..doors.len() {
            for j in (i + 1)..doors.len() {
                assert_ne!(
                    (doors[i].from, doors[i].direction),
                    (doors[j].from, doors[j].direction),
                    "doors {i} and {j} share the same (from, direction)"
                );
            }
        }
    }

    /// G2 — `door(from, direction)` returns each table row verbatim.
    #[test]
    fn room_graph_door_matches_table() {
        let graph = RoomGraph::new();

        for expected in table() {
            assert_eq!(
                graph.door(expected.from, expected.direction),
                Some(expected),
                "door {:?} from {:?}",
                expected.direction,
                expected.from
            );
        }

        // Spec spot-check: (0,0) East.
        assert_eq!(
            graph.door((0, 0), Direction::East),
            Some(Door {
                from: (0, 0),
                direction: Direction::East,
                target: (1, 0),
                trigger: Rect::new(760.0, 320.0, 40.0, 40.0),
                entry: (10.0, 321.0),
            })
        );
    }

    /// G3 — directions with no door in that room return `None`.
    #[test]
    fn room_graph_door_none_for_missing_direction() {
        let graph = RoomGraph::new();

        assert_eq!(graph.door((0, 0), Direction::North), None);
        assert_eq!(graph.door((0, 0), Direction::West), None);
        assert_eq!(graph.door((1, 0), Direction::North), None);
        assert_eq!(graph.door((0, 1), Direction::West), None);
        assert_eq!(graph.door((1, 1), Direction::East), None);
        assert_eq!(graph.door((1, 1), Direction::South), None);
    }

    /// G4 — every door has a reverse counterpart in `target` pointing back.
    #[test]
    fn room_graph_every_door_has_reverse() {
        let graph = RoomGraph::new();

        for door in table() {
            let back = graph
                .door(door.target, opposite(door.direction))
                .expect("reverse door");
            assert_eq!(back.target, door.from);
            assert_eq!(back.direction, opposite(door.direction));
        }
    }

    /// G5 — `can_enter` is true for a 30x38 rect placed just inside each
    /// geometric trigger band, plus the spec's literal (0,0) East case.
    #[test]
    fn room_graph_can_enter_true_at_each_trigger() {
        let graph = RoomGraph::new();

        for door in table() {
            let rect = Rect::new(door.trigger.x + 4.0, door.trigger.y + 1.0, 30.0, 38.0);
            assert!(
                graph.can_enter(door.from, door.direction, rect),
                "can_enter false for {:?} from {:?}",
                door.direction,
                door.from
            );
        }

        assert!(graph.can_enter((0, 0), Direction::East, Rect::new(766.0, 320.0, 30.0, 38.0)));
    }

    /// G6 — the same direction is not enterable when the rect is far away.
    #[test]
    fn room_graph_can_enter_false_away_from_trigger() {
        let graph = RoomGraph::new();

        for door in table() {
            assert!(
                !graph.can_enter(
                    door.from,
                    door.direction,
                    Rect::new(100.0, 200.0, 30.0, 38.0)
                ),
                "can_enter true for {:?} from {:?} at (100, 200)",
                door.direction,
                door.from
            );
        }
    }

    /// G7 — `door_at` reports the door direction inside each band and `None`
    /// in the middle of every room.
    #[test]
    fn room_graph_door_at_returns_direction() {
        let graph = RoomGraph::new();

        for door in table() {
            let inset = Rect::new(door.trigger.x + 4.0, door.trigger.y + 1.0, 30.0, 38.0);
            assert_eq!(
                graph.door_at(door.from, inset).map(|d| d.direction),
                Some(door.direction),
                "door_at for {:?} from {:?}",
                door.direction,
                door.from
            );
        }

        for room in [(0u8, 0u8), (1, 0), (0, 1), (1, 1)] {
            assert_eq!(
                graph.door_at(room, Rect::new(400.0, 240.0, 30.0, 38.0)),
                None,
                "door_at in the middle of room {room:?}"
            );
        }
    }

    /// G8 — each trigger's center tile is `Empty` in the source level.
    /// A point/tile check is used instead of `solid_in_rect(trigger)` because the
    /// trigger's inner edge intentionally shares the floor row.
    #[test]
    fn room_graph_trigger_center_tile_is_empty() {
        let map = Map::new();

        for door in table() {
            let cx = door.trigger.x + door.trigger.w / 2.0;
            let cy = door.trigger.y + door.trigger.h / 2.0;
            let level = level_for(&map, door.from);
            assert_eq!(
                level.grid[(cy / 40.0) as usize][(cx / 40.0) as usize],
                TileType::Empty,
                "trigger center ({cx}, {cy}) is not empty for {:?} from {:?}",
                door.direction,
                door.from
            );
        }
    }

    /// G9 — every canonical entry is in bounds and non-solid in its target.
    #[test]
    fn room_graph_entries_are_in_bounds_and_not_solid() {
        let map = Map::new();

        for door in table() {
            let level = level_for(&map, door.target);
            let (ex, ey) = door.entry;

            assert!(
                ex >= 0.0 && ey >= 0.0,
                "entry ({ex}, {ey}) is negative for {:?} from {:?}",
                door.direction,
                door.from
            );
            assert!(
                ex + 30.0 <= level.pixel_width() && ey + 38.0 <= level.pixel_height(),
                "entry ({ex}, {ey}) is out of bounds for room {:?}",
                door.target
            );
            assert!(
                !level.solid_in_rect(Rect::new(ex, ey, 30.0, 38.0)),
                "entry ({ex}, {ey}) is solid in room {:?}",
                door.target
            );
        }
    }

    /// G10 — `server_trigger` grows only the approach edge by
    /// `DOOR_LEAD_TOLERANCE_PX` (8.0) for all eight doors.
    #[test]
    fn door_server_trigger_expands_approach_edge() {
        let graph = RoomGraph::new();

        let expected = [
            (
                (0u8, 0u8),
                Direction::East,
                Rect::new(752.0, 320.0, 48.0, 40.0),
            ),
            ((1, 0), Direction::West, Rect::new(0.0, 320.0, 48.0, 40.0)),
            (
                (0, 0),
                Direction::South,
                Rect::new(280.0, 432.0, 80.0, 48.0),
            ),
            ((0, 1), Direction::North, Rect::new(280.0, 0.0, 80.0, 48.0)),
            (
                (1, 0),
                Direction::South,
                Rect::new(480.0, 432.0, 80.0, 48.0),
            ),
            ((1, 1), Direction::North, Rect::new(480.0, 0.0, 80.0, 48.0)),
            ((0, 1), Direction::East, Rect::new(752.0, 400.0, 48.0, 40.0)),
            ((1, 1), Direction::West, Rect::new(0.0, 400.0, 48.0, 40.0)),
        ];

        for (from, direction, expected_rect) in expected {
            let door = graph.door(from, direction).expect("door in table");
            let expanded = door.server_trigger();

            assert_eq!(
                expanded, expected_rect,
                "server_trigger for {direction:?} from {from:?}"
            );

            // The non-approach axis must be unchanged.
            match direction {
                Direction::East | Direction::West => {
                    assert_eq!(expanded.y, door.trigger.y);
                    assert_eq!(expanded.h, door.trigger.h);
                }
                Direction::South | Direction::North => {
                    assert_eq!(expanded.x, door.trigger.x);
                    assert_eq!(expanded.w, door.trigger.w);
                }
            }
        }
    }

    /// G11 — `can_enter` accepts a near edge within the lead tolerance and
    /// rejects one beyond it, one boundary per direction.
    #[test]
    fn can_enter_accepts_within_lead_tolerance() {
        let graph = RoomGraph::new();

        // East (0,0): right edge 760, expanded left edge 752.
        assert!(graph.can_enter((0, 0), Direction::East, Rect::new(730.0, 320.0, 30.0, 38.0)));
        assert!(!graph.can_enter((0, 0), Direction::East, Rect::new(700.0, 320.0, 30.0, 38.0)));

        // West (1,0): expanded right edge 48.
        assert!(graph.can_enter((1, 0), Direction::West, Rect::new(42.0, 320.0, 30.0, 38.0)));
        assert!(!graph.can_enter((1, 0), Direction::West, Rect::new(50.0, 320.0, 30.0, 38.0)));

        // South (0,0): expanded top edge 432.
        assert!(graph.can_enter(
            (0, 0),
            Direction::South,
            Rect::new(305.0, 400.0, 30.0, 38.0)
        ));
        assert!(!graph.can_enter(
            (0, 0),
            Direction::South,
            Rect::new(305.0, 390.0, 30.0, 38.0)
        ));

        // North (0,1): expanded bottom edge 48.
        assert!(graph.can_enter((0, 1), Direction::North, Rect::new(305.0, 44.0, 30.0, 38.0)));
        assert!(!graph.can_enter((0, 1), Direction::North, Rect::new(305.0, 50.0, 30.0, 38.0)));
    }

    /// G12 — `door_at` stays on the raw geometric trigger: the exact rects
    /// that `can_enter` accepts within the lead tolerance are `None` here.
    #[test]
    fn door_at_stays_geometric_at_lead_boundary() {
        let graph = RoomGraph::new();

        assert_eq!(
            graph.door_at((0, 0), Rect::new(730.0, 320.0, 30.0, 38.0)),
            None
        );
        assert_eq!(
            graph.door_at((1, 0), Rect::new(42.0, 320.0, 30.0, 38.0)),
            None
        );
        assert_eq!(
            graph.door_at((0, 0), Rect::new(305.0, 400.0, 30.0, 38.0)),
            None
        );
        assert_eq!(
            graph.door_at((0, 1), Rect::new(305.0, 44.0, 30.0, 38.0)),
            None
        );
    }
}
