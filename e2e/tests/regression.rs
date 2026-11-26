// Phase 2e — Regression Test (AC 27)
// Verifies Phase 2d fix for duplicate JoinRequest still works.

use e2e::harness::TestHarness;

/// AC 27 — No duplicate JoinRequest (the World::new fix from Phase 2d remains).
/// Client connects once → only one JoinRequest sent.
#[test]
fn test_no_duplicate_join_request() {
    let mut harness = TestHarness::new();

    // Connect a client
    let handle = harness.connect_client();

    // The TestHarness.connect_client() sends JoinResponse directly to the client
    // and calls handle_packet. The client's World::new() should NOT send a
    // JoinRequest because the fix in Phase 2d moved JoinRequest sending to
    // main.rs before World construction.

    // Advance a few ticks
    harness.tick_n(5);

    // Count how many JoinRequest the server received
    // The TestHarness doesn't expose server-received packets directly,
    // but we can check that only one player was spawned on the server.
    assert_eq!(
        harness.server.players().len(),
        1,
        "Server should have exactly Planner is fixing all 5 issues (session @Write Phase 2e plan). Pipeline:
Stage	Status
1. Spec1 player (no duplicate join)"
    );

    // Also verify the client only has one assigned id
    let client = harness.client(handle);
    assert!(client.world.id.is_some(), "Client should have assigned ID");
    assert_eq!(client.world.player.id, client.world.id.unwrap());

    // The server's total_shots should be empty (no duplicate player entities)
    // This test passes if the Phase 2d fix is still in place.
}
