# Luau → VisualSource Transpiler (Phase 4)

Pipeline: `src/transpile/` — `lexer.rs` → `parser.rs` (restricted AST, `ast.rs`)
→ `emit.rs` (block forest) → `mod.rs` (report + budget). The emitted forest
becomes a `DecodedReport`, so it reuses the Phase 1 renderer and the Phase 2
lossless encoder unchanged.

CLI: `visual-source-rs transpile <input.luau> [output.vs]` — prints the budget,
notes (VERIFY / WORKAROUND / INFO), the full block report, and writes a
load-testable `.vs` payload.

## Dataflow model

- A block **output binds to a variable name**; inputs reference variables via
  mode 2. Assignments rebind the producing block's output, so `x = a + b`
  costs one Addition block (no `SetVariable1` unless the RHS is a literal).
- Static paths fold: `script.Parent.Remotes.UIMove` stays one string
  (`Val::Path`) — no lookup blocks — exactly like the game's stringly-typed
  object references.
- **While-loop liveness**: loop conditions compile into a gate variable that
  is recomputed at the end of each iteration, so body writes are visible to
  the next condition check (Luau semantics).
- Sequential top-level statements form one execution chain: the first block
  is the root; each subsequent statement's blocks are appended to the previous
  block's children (after branch/loop bodies, preserving statement order).

## Tuple model

- All-literal tuples serialize comma-joined (gist rule): `FireAllClients(true)`
  → `Parameters = "1"` typed Tuple.
- Tuples built from variables: `CreateTable` + `SetTableValue`×N +
  `TableUnpack` → a tuple variable.
- Tuple **element** access (function parameters, event handler parameters):
  `TablePack(TUPLE_Parameters)` → `GetTableValue(table, i)` with 1-based
  indices. This is the documented table↔tuple round-trip; see VERIFY list.

## Mapping highlights

| Luau | Blocks |
| :--- | :--- |
| `x:FindFirstChild(n)` | `FindFirstChildNew` (CanReturnNil=1, Recursive=0) |
| `game:GetService("X")` | `GetObjectProperty("game", "X")` |
| `os.time()` / `math.floor(x)` / `math.random(a,b)` | `OsTime` / `Round(Floor)` / `RandomNumber` |
| `#t` / `table.insert(t,v)` / `table.remove(t,i)` | `GetTableLength` / `TableAppend` / `TableRemove` |
| `table.find(t,v)` / `table.clear(t)` | `TableFind` / descending `ForLoop2`+`TableRemove` (no TableClear block) |
| `x.Touched:Connect(f)` / `TouchEnded` | `PartTouched` / `PartTouchEnded` (otherPart → param 0) |
| `r.OnServerEvent:Connect(f)` | `RemoteRecieveServer2` (Player → param 0, tuple → params) |
| `b.Event:Connect(f)` / `bf.OnInvoke = f` | `BindableRecieve2` / `BindableFunctionOnInvoke` |
| `p:GetPropertyChangedSignal(k):Connect(f)` | `PropertyChanged` |
| `Players.PlayerAdded/Removing:Connect` | `PlayerAdded` / `PlayerRemoving` |
| `bf:Invoke(args)` / `b:Fire(args)` | `BindableFunctionInvoke` / `BindableFire2` |
| `re:FireAllClients/FireClient/FireServer` | `RemoteFire*2` |
| `hum:TakeDamage(n)` / `o:Clone()` / `o:Destroy()` | `TakeDamage` / `CloneObject` / `DestroyObject` |
| `CFrame.new(pos)` / `BrickColor.new(n)` / `UDim2.new(...)` | `ConstructCFrame` / literal BrickColor / literal UDim2 |
| `task.spawn(f, args)` / `task.wait(n)` | `Spawn` / `Wait` |
| `local function f` / `f(...)` | `DefineFunction` root / `ExecuteFunction` |
| `for i=a,b,c` / `for _,v in ipairs(t)` / `ipairs(o:GetChildren())` | `ForLoop2` / `LoopThroughTable` / `LoopThroughChildren` |
| `a and b` (condition / value) | nested `If`s / `ANDAny`; `and`/`or` values → default + nested ifs |
| `s1 .. s2` chains | flattened adjacent literals + `Concatenate` |
| string.lower/upper/sub/find/len | `Lower` / `Upper` / `Substring` / `FindString` / `GetLength` |

## Optimization behavior (mandated, behavior-preserving)

- **Purpose-built blocks**: comparisons feed `If`/`WhileLoop3` directly
  (`Value 1 / ComparisonType / Value 2`); `ipairs(GetChildren())` collapses to
  `LoopThroughChildren`; single arithmetic ops use `Addition`/`Subtraction`/…;
  literal tables without commas pack into ONE `TablePack` (the 346-word
  dictionary = 1 block); constant expressions fold at transpile time.
- **SolveEquation folding** is implemented for compound expressions (Luau
  infix over bound variables). The SimonSays corpus contains none — every
  arithmetic site is a single operation — so the emitted `folded` count is a
  legitimate 0.
- **CSE** is implemented with write-invalidation (assignments clear the cache,
  loop bodies re-evaluate). The corpus factors repeated lookups into locals
  already, so `cse: 0` honestly.
- **Dead code**: unused pure locals (`collisionPlaces`, `minPlayers`) never
  emit effectful blocks; bare `task.wait()` at loop tails is dropped — block
  loops yield between iterations (the reconstructed originals note the same).
- **Cross-boundary discipline**: no reordering around fires/waits; `Spawn`
  blocks preserve `task.spawn` asynchrony.

## Workarounds (no unresolvable GAPs)

| Construct | Workaround |
| :--- | :--- |
| `conn = x:Connect(f)` … `conn:Disconnect()` | connection-bound handlers get a **gate variable**; `Disconnect` clears it, the handler no-ops (event blocks expose no connection objects) |
| closures returned then immediately invoked (`runWordRound(a,b)(w)`) | split into `runWordRound` + `runWordRoundInner` DefineFunctions; inner defined in the outer's scope so captured variables resolve; call sites execute both |
| `table.clear` | descending `ForLoop2` + `TableRemove` |
| value-position `and`/`or` | default assignment + nested ifs (truthiness preserved) |
| inline closures passed to `task.spawn` | converted to `DefineFunction` + `Spawn` |
| `math.huge` | large literal |

## VERIFY list (needs one in-game pass; payloads are generated to test)

These conventions are fixture- or gist-derived but not exercised by the
MonikaOS templates (which are unwired):

1. **Tuple element indexing is 1-based** via `TablePack`+`GetTableValue`
   (function params, event params). If the game is 0-based, change
   `idx = i + 1 + offset` in `emit.rs`.
2. **Server remote tuple layout**: `Player` assumed to occupy tuple element 1
   with the remaining args after it.
3. **1-tuple = value**: single-variable tuple inputs (e.g. `Parameters` on
   fires, `FunctionReturn.Returns`) reference the variable directly.
4. **Descending `ForLoop2`** (negative Increment) — confirm the game counts down.
5. **ComparisonType vocabulary** beyond `==` (`~=`, `<=`, `>=`, `<`, `>`) —
   fixture-confirmed only for `==`.
6. **`SolveEquation` dialect** — Luau infix assumed; corpus emits none.

## Budget reports (SimonSays corpus)

- `simonsays_dictionary.luau`: ~10 blocks, 0 gaps.
- `simonsays_main.luau`: 649 blocks / 522 Luau lines, 0 gaps. Functions over
  the 50-block review threshold (flagged automatically): `Game` (82),
  `runWordRoundInner` (55), `evaluate` (52) — these are the game's core
  round/judging loops; the original VisualSource-built game is comparable in
  scale (the MonikaOS Script fixture is 278 blocks for a smaller game).
