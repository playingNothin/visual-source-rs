# Block Catalog Reference

This document is the authoritative coverage report for the converter's block catalog
(`src/vs_blocks.json`), produced by the Phase 3 three-way cross-reference of:

1. **Decoded fixtures** (`fixtures/*.vs`) — ground truth; 556 block instances across
   a Script (278), a LocalScript (277), and a ModuleScript (1), covering **330 distinct types**.
2. **The catalog itself** — `src/vs_blocks.json`, the data-driven block definitions
   (adding a block = adding a JSON entry; no code changes).
3. **The wiki** (`All_Code_Blocks.html`, retro-dev.fandom.com) — *lowest trust*, used only
   as Luau-equivalent documentation. Its own stub banner admits it is incomplete.

Per the source-trust hierarchy: **fixtures win**. Where the catalog or wiki disagreed
with the fixtures, the catalog was fixed; wiki claims about blocks that appear in neither
the fixtures nor the pre-existing catalog were NOT implemented (see
[WIKI_ONLY_UNVERIFIED.md](WIKI_ONLY_UNVERIFIED.md) — the list turned out to be empty).

## Catalog statistics (after Phase 3)

| Metric | Value |
| :--- | :--- |
| Catalog entries | 425 |
| Fixture types covered | 330 / 330 (100%) |
| Categories | Actions 88, Input 47, Events 49, Math 45, Logic 49, Miscellaneous 52, Deprecated 42, Unverified 53 |

## Category semantics

| Category | Meaning | Emission policy |
| :--- | :--- | :--- |
| Actions / Input / Events / Math / Logic / Miscellaneous | Live block groups (fixture-confirmed or pre-existing) | Emit freely |
| Deprecated | Orange blocks; includes the wiki's orange list plus the unsuffixed predecessors of versioned blocks | **NEVER emit** — payloads containing them may be unreadable by the game |
| Unverified | Appears only in the pre-existing catalog; no fixture instance and no reliable wiki row. Includes a layer of pre-Phase-3 guessed names that fixtures now contradict | **NEVER emit** until confirmed by a fixture or ProgrammingBlocksData |

## Conventions

- **Tuple outputs** are stored as a nested list `[["Name"]]`. The game serializes them as
  `TUPLE_Name` bound to `0` (see docs/FORMAT.md §2.2). Single outputs are plain strings.
  Before Phase 3 the catalog mixed both notations inconsistently; all tuple outputs are
  now normalized to the nested form (7 entries fixed).
- **`value_type`** is the declared slot type used for connection matching: `String`,
  `Number`, `Bool`, `Object`, `Vector2/3`, `CFrame`, `UDim2`, `Color3`, `BrickColor`,
  `NumberRange`, `Table`, `Tuple`, `Function`, `AnimationTrack`, `TextChannel`,
  `EventConnection`, `Any`, or a list of alternatives. For the 66 fixture-added entries,
  types were inferred from observed values + input names; entries with an explicitly
  serialized type in a fixture use that type (e.g. `SetNetworkOwner.Player = String`,
  `ModuleReturn.Returns = String`, `TableFind.Value = String`).
- **Input order** in catalog entries matches the fixture field order (ground truth),
  which is occasionally surprising (e.g. `FindFirstChildNew`: CanReturnNil, ChildName,
  Recursive, Parent — CanReturnNil first).

## Catalog fixes applied in Phase 3 (fixture is ground truth)

**Spelling renames** (catalog had never-matchable names):

- `RemoteReceiveServer2` → `RemoteRecieveServer2`, `RemoteReceiveClient2` → `RemoteRecieveClient2`
  (the game misspells "Recieve"; payloads use the misspelled form)

**Input fixes:** `AddToLeaderstat.Value` → `Amount`; `SplitVector2.Vector3` → `Vector2`
(and its type `Vector3` → `Vector2`); `IsGuest` was missing its `Player` input entirely.

**Output fixes:** `LoadInstance.LoadedData` → `LoadedModel`; `OsDate.DataTable` → `DateTable`;
added missing `GetLeaderstat.Value`, `InsertModel.Success`/`LoadedModel`,
`TextBoxFocusLost.EnterPressed`; removed `DumpMemory.Answer`, `ReloadPlace.Result`,
`SetPartPhysicalProperties.PartsTable` (none of these outputs exist in real payloads).

**Output order** normalized to fixture order for 22 types (`SplitVector3`, `InputBegan`,
`FindPartOnRay`, `HumanoidSeated`, …).

**Tuple marking** fixed on 7 entries (`DefineFunction.Parameters`, `BindableFunction*`,
`RemoteFunctionInvoke*`, `RemoteRecieve*2`).

## Full fixture-type coverage table

All 330 types observed in the fixtures. Fixtures column: S=Script, L=LocalScript, M=ModuleScript.
Wiki column reflects whether any wiki row documents this type (the wiki is missing **74**
fixture-confirmed types — its incompleteness runs in the unexpected direction).

| Block type | Fixtures | Catalog | Wiki | Category |
| :--- | :--- | :--- | :--- | :--- |
| `ANDAny` | SL (x2) | yes | yes | Logic |
| `AbortRocketPropulsion` | SL (x2) | yes | yes | Actions |
| `ActivateTool` | SL (x2) | yes | **missing from wiki** | Events |
| `AddDebrisItem` | SL (x2) | yes | yes | Miscellaneous |
| `AddPluginButton` | L (x1) | yes | yes | Miscellaneous |
| `AddPluginToolbar` | L (x1) | yes | yes | Miscellaneous |
| `AddToLeaderstat` | S (x1) | yes | yes | Actions |
| `AddToStudioSelection` | L (x1) | yes | **missing from wiki** | Miscellaneous |
| `AddUserToChatChannel` | S (x1) | yes | yes | Miscellaneous |
| `Addition` | SL (x2) | yes | yes | Math |
| `AncestryChanged` | SL (x2) | yes | **missing from wiki** | Events |
| `Angles` | SL (x2) | yes | yes | Math |
| `AnimationStopped` | SL (x2) | yes | **missing from wiki** | Actions |
| `AnyPropertyChanged` | SL (x2) | yes | yes | Events |
| `Assert` | SL (x2) | yes | yes | Miscellaneous |
| `AssignCollisionGroup` | SL (x2) | yes | yes | Actions |
| `AutowedgeCell` | S (x1) | yes | **missing from wiki** | Actions |
| `BanPlayer` | S (x1) | yes | **missing from wiki** | Actions |
| `BindToClose` | S (x1) | yes | yes | Events |
| `BindableFire2` | SL (x2) | yes | yes | Events |
| `BindableFunctionInvoke` | SL (x2) | yes | **missing from wiki** | Events |
| `BindableFunctionOnInvoke` | SL (x2) | yes | **missing from wiki** | Events |
| `BindableRecieve2` | SL (x2) | yes | yes | Events |
| `BodyPositionReachedTarget` | SL (x2) | yes | yes | Events |
| `BreakJoints` | SL (x2) | yes | yes | Actions |
| `BreakLoop` | SL (x2) | yes | yes | Logic |
| `BrickColorToColor3` | SL (x2) | yes | yes | Math |
| `ChangeHumanoidState` | SL (x2) | yes | yes | Actions |
| `CharacterAdded` | SL (x2) | yes | yes | Events |
| `Chatted` | S (x1) | yes | yes | Events |
| `ChattedSpecificMessage` | S (x1) | yes | yes | Events |
| `CheckIfPlayerIsBanned` | S (x1) | yes | **missing from wiki** | Actions |
| `CheckToolEquipped` | SL (x2) | yes | yes | Actions |
| `ChildAdded` | SL (x2) | yes | yes | Events |
| `ChildRemoved` | SL (x2) | yes | yes | Events |
| `ClickDetectorInteraction` | SL (x2) | yes | yes | Input |
| `CloneObject` | SL (x2) | yes | yes | Actions |
| `CollisionGroupSetCollidable` | SL (x2) | yes | yes | Actions |
| `Color3ToBrickColor` | L (x1) | yes | yes | Math |
| `Color3ToHSV` | SL (x2) | yes | **missing from wiki** | Math |
| `CombineNumberWithString` | SL (x2) | yes | yes | Logic |
| `CombineStringWithNumber` | SL (x2) | yes | yes | Logic |
| `Comment2` | SL (x2) | yes | yes | Miscellaneous |
| `Comparison` | SL (x2) | yes | yes | Logic |
| `ComputePath` | SL (x2) | yes | **missing from wiki** | Actions |
| `Concatenate` | SL (x2) | yes | yes | Logic |
| `ConstructCFrame` | SL (x2) | yes | yes | Math |
| `ConstructColor3` | SL (x2) | yes | yes | Math |
| `ConstructNumberRange` | SL (x2) | yes | yes | Math |
| `ConstructUDim2` | SL (x2) | yes | yes | Math |
| `ConstructVector2` | SL (x2) | yes | yes | Math |
| `ConstructVector3` | SL (x2) | yes | yes | Math |
| `ContinueLoop` | SL (x2) | yes | yes | Logic |
| `CreateChatChannel` | S (x1) | yes | yes | Miscellaneous |
| `CreateLeaderstat` | S (x1) | yes | yes | Actions |
| `CreateObject` | SL (x2) | yes | yes | Actions |
| `CreateTable` | SL (x2) | yes | yes | Logic |
| `DefineFunction` | SL (x2) | yes | yes | Logic |
| `DescendantAdded` | SL (x2) | yes | yes | Events |
| `DescendantRemoved` | SL (x2) | yes | yes | Events |
| `DestroyObject` | SL (x2) | yes | yes | Actions |
| `DialogChat` | SL (x2) | yes | **missing from wiki** | Actions |
| `DialogChoiceSelected` | L (x1) | yes | yes | Events |
| `DisconnectEvent` | SL (x2) | yes | **missing from wiki** | Events |
| `DistanceBetweenPoints` | SL (x2) | yes | yes | Math |
| `Division` | SL (x2) | yes | yes | Math |
| `DoNotRun` | SL (x2) | yes | yes | Logic |
| `DumpMemory` | SL (x2) | yes | yes | Miscellaneous |
| `EmitParticle` | SL (x2) | yes | **missing from wiki** | Actions |
| `Error` | SL (x2) | yes | yes | Miscellaneous |
| `ExecuteBlock` | SL (x2) | yes | yes | Logic |
| `ExecuteFunction` | SL (x2) | yes | yes | Logic |
| `ExplosionHit` | SL (x2) | yes | yes | Events |
| `Exponentiation` | SL (x2) | yes | yes | Math |
| `FilterString` | SL (x2) | yes | yes | Logic |
| `FindFirstChildNew` | SL (x2) | yes | yes | Actions |
| `FindFirstChildOfClassNew` | SL (x2) | yes | yes | Actions |
| `FindFirstChildWhichIsANew` | SL (x2) | yes | yes | Actions |
| `FindPartOnRay` | SL (x2) | yes | yes | Actions |
| `FindPartsInRegion` | SL (x2) | yes | yes | Actions |
| `FindString` | SL (x2) | yes | yes | Logic |
| `FireRocketPropulsion` | SL (x2) | yes | yes | Actions |
| `ForLoop2` | SL (x2) | yes | yes | Logic |
| `FromAxisAngle` | SL (x2) | yes | yes | Math |
| `FromMatrix` | SL (x2) | yes | yes | Math |
| `FunctionReturn` | SL (x2) | yes | yes | Logic |
| `GSubString` | SL (x2) | yes | yes | Logic |
| `GUILeftMouseButtonClick` | SL (x2) | yes | yes | Input |
| `GUILeftMouseButtonDown` | SL (x2) | yes | yes | Input |
| `GUILeftMouseButtonUp` | SL (x2) | yes | yes | Input |
| `GUIMouseEnter` | SL (x2) | yes | yes | Input |
| `GUIMouseLeave` | SL (x2) | yes | yes | Input |
| `GUIMouseMoved` | SL (x2) | yes | yes | Input |
| `GUIMouseWheelBackward` | SL (x2) | yes | yes | Input |
| `GUIMouseWheelForward` | L (x1) | yes | yes | Input |
| `GUIRightMouseButtonClick` | SL (x2) | yes | yes | Input |
| `GUIRightMouseButtonDown` | SL (x2) | yes | yes | Input |
| `GUIRightMouseButtonUp` | SL (x2) | yes | yes | Input |
| `GetCFrameDirection` | SL (x2) | yes | yes | Math |
| `GetCameraPositionLocal` | L (x1) | yes | yes | Actions |
| `GetCell` | SL (x2) | yes | **missing from wiki** | Actions |
| `GetChildren` | SL (x2) | yes | yes | Actions |
| `GetConnectedParts` | SL (x2) | yes | yes | Actions |
| `GetDefaultChannel` | L (x1) | yes | yes | Miscellaneous |
| `GetDescendants` | SL (x2) | yes | yes | Actions |
| `GetFullName` | SL (x2) | yes | yes | Actions |
| `GetGameBadges` | S (x1) | yes | yes | Miscellaneous |
| `GetGameInfo` | S (x1) | yes | yes | Miscellaneous |
| `GetGamepasses` | S (x1) | yes | yes | Miscellaneous |
| `GetHumanoidState` | SL (x2) | yes | yes | Actions |
| `GetJoints` | SL (x2) | yes | yes | Actions |
| `GetLeaderstat` | SL (x2) | yes | yes | Actions |
| `GetLength` | SL (x2) | yes | yes | Logic |
| `GetModelBoundingBox` | SL (x2) | yes | yes | Actions |
| `GetMousePosition2D` | S (x1) | yes | **missing from wiki** | Input |
| `GetMousePosition2DLocal` | L (x1) | yes | yes | Input |
| `GetMousePosition3D` | S (x1) | yes | **missing from wiki** | Input |
| `GetMousePosition3DLocal` | L (x1) | yes | yes | Input |
| `GetMouseTargetPart` | S (x1) | yes | **missing from wiki** | Input |
| `GetMouseTargetPartLocal` | L (x1) | yes | yes | Input |
| `GetMouseTargetSurface` | S (x1) | yes | **missing from wiki** | Input |
| `GetMouseTargetSurfaceLocal` | L (x1) | yes | yes | Input |
| `GetNameFromUserIdAsync` | SL (x2) | yes | yes | Actions |
| `GetObjectProperty` | SL (x2) | yes | yes | Actions |
| `GetPartDirection` | SL (x2) | yes | yes | Math |
| `GetPartPhysicalProperties` | SL (x2) | yes | yes | Actions |
| `GetPlayerFromCharacter` | SL (x2) | yes | yes | Actions |
| `GetPlayersOnTeam` | SL (x2) | yes | **missing from wiki** | Actions |
| `GetRandomChild` | SL (x2) | yes | yes | Actions |
| `GetRankInGroup` | SL (x2) | yes | yes | Miscellaneous |
| `GetRobloxVersion` | SL (x2) | yes | yes | Miscellaneous |
| `GetServerTimeNow` | SL (x2) | yes | yes | Miscellaneous |
| `GetStudioSelection` | L (x1) | yes | yes | Miscellaneous |
| `GetStudioTheme` | L (x1) | yes | **missing from wiki** | Miscellaneous |
| `GetTableLength` | SL (x2) | yes | yes | Logic |
| `GetTableValue` | SL (x2) | yes | yes | Logic |
| `GetTestStatus` | L (x1) | yes | yes | Miscellaneous |
| `GetTouchingParts` | SL (x2) | yes | yes | Actions |
| `GetUserIdFromNameAsync` | SL (x2) | yes | yes | Actions |
| `GetUserThumbnail` | SL (x2) | yes | **missing from wiki** | Miscellaneous |
| `GiveBadge` | S (x1) | yes | yes | Actions |
| `HSVToColor3` | SL (x2) | yes | **missing from wiki** | Math |
| `HandlesMouseButtonDown` | L (x1) | yes | **missing from wiki** | Input |
| `HandlesMouseButtonUp` | L (x1) | yes | **missing from wiki** | Input |
| `HandlesMouseDrag` | L (x1) | yes | **missing from wiki** | Input |
| `HandlesMouseEnter` | L (x1) | yes | **missing from wiki** | Input |
| `HandlesMouseLeave` | L (x1) | yes | **missing from wiki** | Input |
| `HasBadge` | S (x1) | yes | yes | Actions |
| `HasBadgeFromForeignPlace` | S (x1) | yes | **missing from wiki** | Actions |
| `HasGamepass` | S (x1) | yes | yes | Actions |
| `HasGamepassFromForeignPlace` | S (x1) | yes | **missing from wiki** | Actions |
| `Heartbeat` | SL (x2) | yes | yes | Events |
| `HumanoidDied` | SL (x2) | yes | yes | Events |
| `HumanoidFreeFalling` | SL (x3) | yes | yes | Events |
| `HumanoidJumping` | SL (x2) | yes | yes | Events |
| `HumanoidMoveTo` | SL (x2) | yes | **missing from wiki** | Actions |
| `HumanoidMoveToFinished` | SL (x2) | yes | **missing from wiki** | Events |
| `HumanoidRunning` | SL (x2) | yes | yes | Events |
| `HumanoidSeated` | SL (x2) | yes | yes | Events |
| `HumanoidStateChanged` | SL (x2) | yes | yes | Events |
| `If` | SL (x2) | yes | yes | Logic |
| `InputBegan` | L (x1) | yes | **missing from wiki** | Input |
| `InputChanged` | L (x1) | yes | **missing from wiki** | Input |
| `InputEnded` | L (x1) | yes | **missing from wiki** | Input |
| `InsertModel` | SL (x2) | yes | yes | Miscellaneous |
| `InsertRetrostudioAvatar` | S (x1) | yes | yes | Miscellaneous |
| `InverseCFrame1` | SL (x2) | yes | yes | Math |
| `IsA` | SL (x2) | yes | yes | Actions |
| `IsAncestorOf` | SL (x2) | yes | yes | Actions |
| `IsClient` | SL (x2) | yes | **missing from wiki** | Miscellaneous |
| `IsConnected` | SL (x2) | yes | **missing from wiki** | Events |
| `IsDescendantOf` | SL (x2) | yes | yes | Actions |
| `IsFriendsWith` | S (x1) | yes | yes | Miscellaneous |
| `IsGameLoaded` | SL (x2) | yes | yes | Miscellaneous |
| `IsGuest` | SL (x2) | yes | yes | Miscellaneous |
| `IsInGroup` | SL (x2) | yes | yes | Miscellaneous |
| `IsOwner` | SL (x2) | yes | **missing from wiki** | Actions |
| `IsPlayingSolo` | SL (x2) | yes | yes | Miscellaneous |
| `IsServer` | SL (x3) | yes | **missing from wiki** | Miscellaneous |
| `IsStudio` | SL (x2) | yes | yes | Miscellaneous |
| `IsTouching` | SL (x2) | yes | yes | Actions |
| `IsTouchscreen` | L (x1) | yes | yes | Miscellaneous |
| `JsonDecode` | L (x1) | yes | yes | Miscellaneous |
| `JsonEncode` | SL (x2) | yes | yes | Miscellaneous |
| `JumpRequest` | L (x1) | yes | **missing from wiki** | Input |
| `KeyDown` | S (x1) | yes | **missing from wiki** | Input |
| `KeyDownLocal` | L (x1) | yes | yes | Input |
| `KeyUp` | S (x1) | yes | **missing from wiki** | Input |
| `KeyUpLocal` | L (x1) | yes | yes | Input |
| `KickPlayer` | S (x1) | yes | yes | Actions |
| `LeaderstatChanged` | SL (x2) | yes | yes | Actions |
| `LeftMouseDown` | S (x1) | yes | **missing from wiki** | Input |
| `LeftMouseDownLocal` | L (x1) | yes | yes | Input |
| `LeftMouseUp` | S (x1) | yes | **missing from wiki** | Input |
| `LeftMouseUpLocal` | L (x1) | yes | yes | Input |
| `Lerp1` | SL (x2) | yes | yes | Math |
| `LoadAnimation` | SL (x2) | yes | **missing from wiki** | Actions |
| `LoadCharacter` | S (x1) | yes | yes | Actions |
| `LoadInstance` | S (x1) | yes | yes | Miscellaneous |
| `LoadVariable` | S (x1) | yes | yes | Miscellaneous |
| `LoopThroughChildren` | SL (x2) | yes | yes | Logic |
| `LoopThroughDescendants` | SL (x2) | yes | yes | Logic |
| `LoopThroughTable` | SL (x3) | yes | yes | Logic |
| `Lower` | SL (x2) | yes | yes | Logic |
| `MakeJoints` | SL (x2) | yes | yes | Actions |
| `MathLibraryDoubleInput` | SL (x2) | yes | yes | Math |
| `MathLibrarySingleInput` | SL (x3) | yes | yes | Math |
| `MathLibraryTripleInput` | SL (x2) | yes | yes | Math |
| `ModelInserted` | S (x1) | yes | yes | Miscellaneous |
| `ModuleReturn` | M (x1) | yes | **missing from wiki** | Logic |
| `Modulus` | SL (x2) | yes | yes | Math |
| `MoveModelTo` | SL (x2) | yes | yes | Actions |
| `Multiplication` | SL (x2) | yes | yes | Math |
| `MutePlayer` | S (x1) | yes | **missing from wiki** | Actions |
| `NOTAny` | SL (x2) | yes | yes | Logic |
| `NormalizeVector3` | SL (x2) | yes | yes | Math |
| `ORAny` | SL (x2) | yes | yes | Logic |
| `OsClock` | SL (x2) | yes | yes | Miscellaneous |
| `OsDate` | SL (x2) | yes | yes | Miscellaneous |
| `OsTime` | SL (x2) | yes | yes | Miscellaneous |
| `PartTouchEnded` | SL (x2) | yes | yes | Events |
| `PartTouched` | SL (x2) | yes | yes | Events |
| `PauseSound` | SL (x2) | yes | yes | Actions |
| `PerlinNoise` | SL (x2) | yes | yes | Math |
| `PlayAnimation` | SL (x2) | yes | **missing from wiki** | Actions |
| `PlaySound` | SL (x2) | yes | yes | Actions |
| `PlayerAdded` | SL (x2) | yes | yes | Events |
| `PlayerRemoving` | SL (x2) | yes | yes | Events |
| `PluginButtonClicked` | L (x1) | yes | yes | Miscellaneous |
| `PluginButtonSetActive` | L (x1) | yes | yes | Miscellaneous |
| `PluginDeactivating` | SL (x2) | yes | yes | Miscellaneous |
| `PreSimulation` | SL (x2) | yes | **missing from wiki** | Events |
| `Print` | SL (x2) | yes | yes | Miscellaneous |
| `PromptDeveloperProductPurchase` | SL (x2) | yes | **missing from wiki** | Actions |
| `PromptDeveloperProductPurchaseFinished` | S (x1) | yes | **missing from wiki** | Actions |
| `PromptGamepassPurchase` | SL (x2) | yes | **missing from wiki** | Actions |
| `PromptGamepassPurchaseFinished` | S (x1) | yes | **missing from wiki** | Actions |
| `PropertyChanged` | SL (x2) | yes | yes | Events |
| `RandomBrickColor` | SL (x2) | yes | yes | Math |
| `RandomNumber` | SL (x2) | yes | yes | Math |
| `ReceiveChatMessage` | L (x1) | yes | yes | Miscellaneous |
| `RegisterCollisionGroup` | S (x1) | yes | yes | Actions |
| `ReloadPlace` | S (x1) | yes | yes | Miscellaneous |
| `RemoteFireAllClients2` | S (x1) | yes | yes | Events |
| `RemoteFireClient2` | S (x1) | yes | yes | Events |
| `RemoteFireServer2` | L (x1) | yes | yes | Events |
| `RemoteFunctionInvokeClient` | S (x1) | yes | **missing from wiki** | Events |
| `RemoteFunctionInvokeServer` | L (x1) | yes | **missing from wiki** | Events |
| `RemoteFunctionOnInvokeClient` | L (x1) | yes | **missing from wiki** | Events |
| `RemoteFunctionOnInvokeServer` | S (x1) | yes | **missing from wiki** | Events |
| `RemoteRecieveClient2` | L (x1) | yes | **missing from wiki** | Events |
| `RemoteRecieveServer2` | S (x1) | yes | yes | Events |
| `RemoveFromStudioSelection` | L (x1) | yes | **missing from wiki** | Miscellaneous |
| `RenderStepped` | L (x1) | yes | yes | Events |
| `Require` | SL (x2) | yes | yes | Logic |
| `ResumeSound` | SL (x2) | yes | yes | Actions |
| `RightMouseDown` | S (x1) | yes | **missing from wiki** | Input |
| `RightMouseDownLocal` | L (x1) | yes | yes | Input |
| `RightMouseUp` | S (x1) | yes | **missing from wiki** | Input |
| `RightMouseUpLocal` | L (x1) | yes | yes | Input |
| `RotationBetweenPoints` | SL (x2) | yes | yes | Math |
| `Round` | SL (x2) | yes | yes | Math |
| `SaveInstance` | S (x1) | yes | yes | Miscellaneous |
| `SaveVariable` | S (x1) | yes | yes | Miscellaneous |
| `SendChatMessage` | L (x1) | yes | yes | Miscellaneous |
| `SetCameraPositionLocal` | L (x1) | yes | yes | Actions |
| `SetCell` | S (x1) | yes | **missing from wiki** | Actions |
| `SetCoreGuiEnabled` | L (x1) | yes | yes | Actions |
| `SetLeaderstat` | S (x1) | yes | yes | Actions |
| `SetModelPrimaryPartCFrameNew` | SL (x2) | yes | yes | Actions |
| `SetMouseBehavior` | L (x1) | yes | **missing from wiki** | Input |
| `SetMouseIconLocal` | L (x1) | yes | yes | Actions |
| `SetMouseTargetFilterLocal` | L (x1) | yes | yes | Input |
| `SetNetworkOwner` | SL (x2) | yes | yes | Actions |
| `SetObjectProperty` | SL (x2) | yes | yes | Actions |
| `SetPartPhysicalProperties` | SL (x2) | yes | yes | Actions |
| `SetTableValue` | SL (x2) | yes | yes | Logic |
| `SetVariable1` | SL (x2) | yes | yes | Logic |
| `SolveEquation` | SL (x2) | yes | yes | Math |
| `SortTable` | SL (x2) | yes | yes | Logic |
| `SortTableAdvanced` | SL (x2) | yes | yes | Logic |
| `SortTableAdvancedReturn` | SL (x2) | yes | yes | Logic |
| `Spawn` | SL (x2) | yes | yes | Logic |
| `SplitCFrame` | SL (x2) | yes | yes | Math |
| `SplitColor3` | SL (x2) | yes | yes | Math |
| `SplitNumberRange` | SL (x2) | yes | yes | Math |
| `SplitString` | SL (x2) | yes | yes | Logic |
| `SplitUDim2` | SL (x2) | yes | yes | Math |
| `SplitVector2` | SL (x2) | yes | yes | Math |
| `SplitVector3` | SL (x2) | yes | yes | Math |
| `StopAnimation` | SL (x2) | yes | **missing from wiki** | Actions |
| `StopSound` | SL (x2) | yes | yes | Actions |
| `StudioSelectionChanged` | L (x1) | yes | **missing from wiki** | Events |
| `Substring` | SL (x2) | yes | yes | Logic |
| `Subtraction` | SL (x2) | yes | yes | Math |
| `TableAppend` | SL (x2) | yes | **missing from wiki** | Logic |
| `TableFind` | SL (x2) | yes | **missing from wiki** | Logic |
| `TableInsert` | SL (x2) | yes | yes | Logic |
| `TablePack` | SL (x2) | yes | **missing from wiki** | Logic |
| `TableRemove` | SL (x2) | yes | yes | Logic |
| `TableUnpack` | SL (x2) | yes | **missing from wiki** | Logic |
| `TakeDamage` | SL (x2) | yes | yes | Actions |
| `TeleportPlayerToPosition` | S (x1) | yes | yes | Actions |
| `TeleportToPlace` | S (x1) | yes | yes | Actions |
| `TeleportToPlaceProtected` | S (x1) | yes | **missing from wiki** | Actions |
| `TestStatusChanged` | L (x1) | yes | yes | Miscellaneous |
| `TextBoxCaptureFocus` | L (x1) | yes | **missing from wiki** | Input |
| `TextBoxFocusLost` | L (x1) | yes | yes | Input |
| `TextBoxFocused` | L (x1) | yes | yes | Input |
| `TextBoxReleaseFocus` | L (x1) | yes | **missing from wiki** | Input |
| `ToEulerAnglesXYZ` | SL (x2) | yes | yes | Math |
| `ToNumberAny` | SL (x2) | yes | yes | Logic |
| `ToObjectSpace1` | SL (x2) | yes | yes | Math |
| `ToStringAny` | SL (x2) | yes | yes | Logic |
| `ToWorldSpace1` | SL (x2) | yes | yes | Math |
| `ToggleMobileControls` | L (x1) | yes | **missing from wiki** | Actions |
| `ToolActivated` | SL (x2) | yes | yes | Events |
| `ToolDeactivated` | SL (x2) | yes | yes | Events |
| `ToolEquipped` | SL (x2) | yes | yes | Events |
| `ToolUnequipped` | SL (x2) | yes | yes | Events |
| `TweenObjectProperty` | SL (x2) | yes | yes | Actions |
| `TypeOf` | SL (x2) | yes | yes | Logic |
| `UnbanPlayerUID` | S (x1) | yes | **missing from wiki** | Actions |
| `Upper` | SL (x2) | yes | yes | Logic |
| `Vector3Cross` | SL (x2) | yes | yes | Math |
| `Vector3Dot` | SL (x2) | yes | yes | Math |
| `Wait` | SL (x2) | yes | yes | Logic |
| `WaitForChild` | SL (x2) | yes | yes | Actions |
| `Warn` | SL (x2) | yes | yes | Miscellaneous |
| `WhileLoop3` | SL (x2) | yes | yes | Logic |

## Catalog-only entries (never in fixtures)

### Deprecated (42) — never emit

`AND`, `BindableFire`, `BindableReceive`, `ClearJoinAfterMoveJoints`, `Comment`, `CreateJoinAfterMoveJoints`, `DescendantRemoving`, `FindFirstChild`, `ForLoop`, `GetModelThumbnail`, `GiveRetrobux`, `InversePositionAndRotation`, `LegacyLeaderstatsSaverConversion`, `Lerp`, `MathPi`, `MegaBlock`, `NOT`, `OR`, `PowerlangImport`, `PowerlangImport2`, `RBXGuiCreateDropDownMenu`, `RBXGuiCreateScrollingFrame`, `RemoteFireAllClients`, `RemoteFireClient`, `RemoteFireServer`, `RemoteReceiveClient`, `RemoteReceiveServer`, `ReverseMortalityEngine`, `SetBodyGyroDirection`, `SetJoinAfterMoveTarget`, `SetModelPrimaryPartCFrame`, `SetVariable`, `ShowPermissibleJoints`, `ToNumber`, `ToObjectSpace`, `ToString`, `ToWorldSpace`, `TweenGuiPosition`, `TweenGuiSize`, `UnbanPlayer`, `WhileLoop`, `WhileLoop2`

Notably: `SetVariable`, `WhileLoop`, `ForLoop`, `FindFirstChild`, `BindableFire`,
`BindableReceive`, and the unsuffixed `RemoteFire*`/`RemoteReceive*` — the wiki's orange
list maps 1:1 onto the predecessors of the current versioned blocks.

### Unverified (53) — never emit until confirmed

These entries existed in the inherited catalog but have **no fixture instance** and no
reliable wiki row. Many are pre-Phase-3 name guesses that fixtures now contradict:

| Catalog entry | Fixture-confirmed counterpart (evidence) |
| :--- | :--- |
| `AddButton` | `AddPluginButton` |
| `AddItemToDebris` | `AddDebrisItem` |
| `AddToSelection` | `AddToStudioSelection` |
| `AddToolbar` | `AddPluginToolbar` |
| `AutoWedgeCell` | `AutowedgeCell` |
| `BindableReceive2` | `BindableRecieve2` |
| `ButtonClicked` | `PluginButtonClicked` |
| `CFrameAngles` | `Angles` |
| `CombineStrings` | `Concatenate` |
| `FindFirstChildOfClass` | `FindFirstChildOfClassNew` |
| `FindFirstChildWhichIsA` | `FindFirstChildWhichIsANew` |
| `FindInTable` | `TableFind` |
| `FocusOnTextbox` | `TextBoxCaptureFocus` |
| `GUILeftClick` | `GUILeftMouseButtonClick` |
| `GUILeftMouseDown` | `GUILeftMouseButtonDown` |
| `GUILeftMouseUp` | `GUILeftMouseButtonUp` |
| `GUIMouseWheelDown` | `GUIMouseWheelBackward` |
| `GUIMouseWheelUp` | `GUIMouseWheelForward` |
| `GUIRightClick` | `GUIRightMouseButtonClick` |
| `GUIRightMouseDown` | `GUIRightMouseButtonDown` |
| `GUIRightMouseUp` | `GUIRightMouseButtonUp` |
| `GetCameraPosition` | `GetCameraPositionLocal` |
| `GetLengthOfString` | `GetLength` |
| `GetNameFromUserId` | `GetNameFromUserIdAsync` |
| `GetSelection` | `GetStudioSelection` |
| `GetUserIdFromName` | `GetUserIdFromNameAsync` |
| `GlobalSubstring` | `GSubString` |
| `HandlesMouseDown` | `HandlesMouseButtonDown` |
| `HandlesMouseUp` | `HandlesMouseButtonUp` |
| `InsertRetroStudioAvatar` | `InsertRetrostudioAvatar` |
| `InverseCFrame` | `InverseCFrame1` |
| `IsPlayerBanned` | `CheckIfPlayerIsBanned` |
| `IsSolo` | `IsPlayingSolo` |
| `Lerp2` | `Lerp1` |
| `LoopThroughDescendant` | `LoopThroughDescendants` |
| `ModelMoveTo` | `MoveModelTo` |
| `PostSimulation` | (no fixture counterpart; plausible but unconfirmed) |
| `PreRender` | (no fixture counterpart; plausible but unconfirmed) |
| `ReceiveRemoteFunction` | (no fixture counterpart; plausible but unconfirmed) |
| `RemoveFromSelection` | `RemoveFromStudioSelection` |
| `SelectionChanged` | `StudioSelectionChanged` |
| `SetButtonActive` | `PluginButtonSetActive` |
| `SetCameraPosition` | `SetCameraPositionLocal` |
| `SetHumanoidState` | (no fixture counterpart; plausible but unconfirmed) |
| `SetMouseBehaviour` | `SetMouseBehavior` |
| `SetMouseIcon` | `SetMouseIconLocal` |
| `SetMouseTargetFilter` | `SetMouseTargetFilterLocal` |
| `SetPartNetworkOwner` | `SetNetworkOwner` |
| `SplitStrings` | `SplitString` |
| `TeleportToOwnedPlace` | `TeleportToPlaceProtected` |
| `UnfocusOnTextbox` | `TextBoxReleaseFocus` |
| `UserJumpRequest` | `JumpRequest` |
| `randomNumber` | `RandomNumber` |

Entries without a suspected counterpart (`PostSimulation`, `PreRender`, `ReceiveRemoteFunction`, `SetHumanoidState`) may be real blocks our fixtures simply
never exercise; they stay in the catalog as inert data and are excluded from emission
until a fixture or ProgrammingBlocksData confirms them.

## Enforcement

`tests/catalog.rs` fails the build if any fixture block type is missing from the catalog,
if its input/output names (or tuple marking) drift from the fixture, if a fixture ever
uses a `Deprecated` type, or if the JSON schema breaks.

