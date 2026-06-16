// src/network.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeerType { Orchestrator, Broker }

// On simule tes CustomServerPackets
#[derive(Debug, Clone, Copy)]
pub enum SimulationPacket {
    // GameServer -> Spatial
    PlayerJoinUpdate { client_id: u32, x: f32, y: f32 },
    PositionUpdate { client_id: u32, x: f32, y: f32 },
    ClientLeft { client_id: u32 },
    ServerHeartBeat { shard_id: u32, occupancy: u32 },
    ServerSpawned { shard_id: u32 },
    HandoffAccept { shard_id: u32, client_id: u32 },

    // Spatial -> GameServer
    SpawnServer { shard_id: u32, x: f32, y: f32 },
    ShutdownServerOnEmpty { shard_id: u32 },
    HandoffRequest { shard_id: u32, client_id: u32 }, // Demande de Ghost
    HandoffComplete { new_shard_id: u32, old_shard_id: u32, client_id: u32 }, // Transfert
    HandoffDrop { shard_id: u32, client_id: u32 }, // Drop Ghost
    SpawnPlayerShard { shard_id: u32, client_id: u32 },
    DespawnPlayerShard { shard_id: u32, client_id: u32 },
}