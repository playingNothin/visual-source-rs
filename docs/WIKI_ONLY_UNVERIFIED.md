# Wiki-Only Unverified Blocks

Per the source-trust hierarchy, any block that appears **only** in the wiki
(`Scripting/All_Code_Blocks`) and in neither the decoded fixtures nor the block
catalog must be reported here and must NOT be implemented.

## Result: empty (0 blocks)

All 266 usable wiki rows (the raw page contains 270 table rows; 4 are legend
rows — "All", "Local Scripts", "Regular Scripts", "Usage Type") were mapped to
block types that are already confirmed by the fixtures or the pre-existing
catalog. No wiki row references a block type absent from both.

### Mapping notes

The wiki uses display names ("Find First Child", "Global Sub String"), not
internal type identifiers. Mapping to internal types required 5 exact-match
corrections beyond normalization:

| Wiki display name | Internal type |
| :--- | :--- |
| AND Gate | `ANDAny` |
| NOT Gate | `NOTAny` |
| OR Gate | `ORAny` |
| Exponentation | `Exponentiation` |
| Humanoid Seating | `HumanoidSeated` |

The wiki's orange/Deprecated section lists 8 blocks, all mapping to
predecessors of current versioned blocks: Set Variable → `SetVariable` (dep.),
While Loop → `WhileLoop`/`WhileLoop2` (dep.), For Loop → `ForLoop` (dep.),
Find First Child → `FindFirstChild` (dep.), Fire/Receive Remote Event →
`RemoteFire*`/`RemoteReceive*` (dep.), Fire/Receive Bindable Event →
`BindableFire`/`BindableReceive` (dep.). All are marked `Deprecated` in the
catalog and never emitted.

### The wiki's incompleteness runs the other way

The wiki is missing **74** of the 330 fixture-confirmed block types — including
ones the game clearly supports and fixtures prove: `TableFind`, `TablePack`,
`TableUnpack`, `TableAppend`, `HumanoidMoveTo`/`HumanoidMoveToFinished`,
`InputBegan`/`InputChanged`/`InputEnded`, `KeyDown`/`KeyUp`, `ComputePath`,
`LoadAnimation`/`PlayAnimation`/`StopAnimation`, `IsServer`/`IsClient`,
`GetPlayersOnTeam`, `RemoteFunctionInvoke*`, `BindableFunction*`, and more.
See the "missing from wiki" rows in [BLOCK_CATALOG.md](BLOCK_CATALOG.md).

This matches the wiki's own stub banner ("This section needs to be expanded")
and the brief's warning: never treat "absent from wiki" as "absent from the
game", and never treat the wiki's Luau-equivalent column as semantic ground
truth.
