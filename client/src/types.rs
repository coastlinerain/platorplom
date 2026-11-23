#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TileType {
    Empty,
    Wall,
    EnemySpawn,
    BossSpawn,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntityType {
    Empty,
    EnemySpawn,
    BossSpawn,
    PotionSpawn,
}

impl From<u8> for TileType {
    fn from(value: u8) -> Self {
        match value {
            1 => TileType::Wall,
            2 => TileType::EnemySpawn,
            3 => TileType::BossSpawn,
            _ => TileType::Empty, // El 0 y cualquier otro caen aquí
        }
    }
}

impl From<u8> for EntityType {
    fn from(value: u8) -> Self {
        match value {
            1 => EntityType::EnemySpawn,
            2 => EntityType::BossSpawn,
            3 => EntityType::PotionSpawn,
            _ => EntityType::Empty, // El 0 y cualquier otro caen aquí
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EntityType, TileType};

    #[test]
    fn tile_type_from_u8() {
        assert_eq!(TileType::from(0u8), TileType::Empty);
        assert_eq!(TileType::from(1u8), TileType::Wall);
        assert_eq!(TileType::from(2u8), TileType::EnemySpawn);
        assert_eq!(TileType::from(3u8), TileType::BossSpawn);
        assert_eq!(TileType::from(4u8), TileType::Empty);
        assert_eq!(TileType::from(255u8), TileType::Empty);
    }

    #[test]
    fn entity_type_from_u8() {
        assert_eq!(EntityType::from(0u8), EntityType::Empty);
        assert_eq!(EntityType::from(1u8), EntityType::EnemySpawn);
        assert_eq!(EntityType::from(2u8), EntityType::BossSpawn);
        assert_eq!(EntityType::from(3u8), EntityType::PotionSpawn);
        assert_eq!(EntityType::from(4u8), EntityType::Empty);
        assert_eq!(EntityType::from(255u8), EntityType::Empty);
    }
}
