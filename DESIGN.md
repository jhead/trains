# Design Brief — Working Title: *Rail Town*

## Vision

A calm, pixel-art sandbox about laying railway by hand and watching a small town grow around the service you provide. Rail is the only thing you build directly. The town is the readout — it thickens where your trains reach and thins where they don't. Scale stays small enough that individual residents remain visible and knowable.

## The Loop

- **Seconds** — place a piece of track. Fast, reversible, immediately legible.
- **A minute** — a train completes a run and pays out.
- **Ten minutes** — a served area thickens; a new demand appears somewhere you can't yet reach.
- **An hour** — the map is threaded, the town has a shape, and that shape is a record of your decisions.

Every tier feeds the one above it. The whole game is: reach something, serve it, watch it grow, need to reach further.

## Track and Terrain

- Track is laid piece by piece while the world runs. Building is an activity, not a menu.
- Terrain generates the puzzle. Valleys, ridges, water and elevation mean every map asks a different routing question.
- Gradient limits, curve radius trading against speed, expensive tunnels, and limited bridge spans keep *shortest*, *cheapest*, and *fastest* as three different routes.
- Underground and elevated construction are available through a layer view — the same map seen at a different depth.
- Long straight runs between two placed pieces can be auto-filled, so hand-building stays where the interest is.
- Demolition refunds in full. Building is free to experiment with; only commitment costs.

## The Town and Its People

- Town scale, not city scale. Residents are individually visible, named, and knowable.
- Peeps have moods and opinions and voice them publicly — "waited eleven minutes at Eastgate." The complaint feed is both the diagnostic layer and the emotional hook.
- Growth is local and caused. A district thickens because a line reached it. Nothing grows on a hidden global counter.
- When service degrades, people leave and buildings empty. This is recoverable at any point by fixing the underlying route.

## Trains

Two types, as sidegrades with distinct constraint profiles:

- **Transit** — moves people. Dense, short hops, stops frequently, drives residential and commercial growth.
- **Transport** — moves goods. Point-to-point between industries, longer runs, funds the network.

Both share one track system. Mixing them on the same corridor is a legitimate strategy and a legitimate mistake.

## Money

- Income comes from throughput. Costs come from construction and from operating expenses that scale with how much network you're running.
- A well-run network outruns its costs comfortably, and that surplus is meant to be spent expanding.
- Overextension inverts it: track that isn't carrying enough starts costing more than it earns. The response is to prune and rebalance, not to start over.
- Money paces expansion. It never ends the game.

## Pressure

- **Congestion** is the standing puzzle. Capacity is finite, trains queue, and a busy corridor is a design problem rather than a disaster.
- **Events** target the network, not the town — a landslide closes a tunnel, a festival spikes demand at one station, a new industry opens off your lines, flooding takes a bridge. Each forces a reroute and rewards networks built with slack and redundancy.
- **Stagnation** is the only failure. A quiet town is the worst outcome, and it stays fixable.

## Modes

- **Sandbox** — the systems with no clock and no target.
- **Goals** — the same systems with objectives like population thresholds or delivery quotas, and deadlines that make the pacing bite.

## Neighbors

Maps sit next to other players' maps. Tunnels at the map edge connect to a neighbor's network; trains cross over carrying goods and come back with theirs. Contributions are additive — a neighbor's line makes your town better, and a neighbor's absence never blocks it. This works asynchronously, so a single-player town is still connected to somebody.
