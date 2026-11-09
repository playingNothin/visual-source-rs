# VisualSource V4 Format Specification

This document is the authoritative format specification for RetroStudio's VisualSource V4 node/block scripting format, derived directly from decoded ground-truth fixtures (`Script`, `LocalScript`, `ModuleScript`), `python.py`, and the V4 format documentation.

---

## 1. Payload Pipeline

VisualSource stores scripts as compressed, ASCII-encoded strings:

```
[Raw VisualSource Text]
        │
        ▼ (libdeflate DEFLATE level 6, raw stream - no zlib/gzip headers)
 [DEFLATE Bytes]
        │
        ▼ (Base93 binary-to-text encoding)
  [Base93 Text]
        │
        ▼ (Prefix with header " 0000000000000004" or "\x1a0000000000000004\x1b")
 [Final .vs String]
```

### 1.1 Header
* **Version 4 Prefix**: `0000000000000004` (16 ASCII digits).
* In export pastes, it may be preceded by ASCII SUB (`\x1a`) and followed by ASCII ESC (`\x1b`) or spaces.
* When decoding, line breaks (`\r`, `\n`) and extraneous chat-wrapping whitespace must be defensively stripped.

### 1.2 Base93 Encoding
Base93 uses a 93-character alphabet (note the trailing space at index 92):
```
ABCDEFGHIJKLMNOPQRSTUVWXYZ
abcdefghijklmnopqrstuvwxyz
0123456789
!#$%&'()*+,-./
:;<=>?@
[]^_`
{|}~ 
```
* **Encoding Rule**:
  * Bits are packed into a 32-bit accumulator.
  * When $\ge 14$ bits are available, the lower 13 bits ($v_{13} = b \ \& \ 8191$) are checked.
  * If $v_{13} > 456$: emit 13 bits as 2 characters ($v = v_{13}$; advance 13 bits).
  * If $v_{13} \le 456$: emit 14 bits as 2 characters ($v = b \ \& \ 16383$; advance 14 bits).
  * Characters: `c1 = ALPHABET[v % 93]`, `c2 = ALPHABET[v / 93]`.
  * Tail handling: remaining bits ($n > 0$) emit `ALPHABET[b % 93]`, and if $n > 7$ or $b > 92$, a second character `ALPHABET[b / 93]`.
* **Decoding Rule**:
  * For pairs $(c_1, c_2)$: $v = \text{val}(c_1) + \text{val}(c_2) \times 93$.
  * Bit count advance: $13$ if $(v \pmod{8192}) > 456$, else $14$.
  * Extract 8-bit bytes while bit count $> 7$.
  * Tail single character $(c)$: add $c \ll \text{bitcnt}$ and extract final byte.

### 1.3 DEFLATE Compression
* **Algorithm**: Raw DEFLATE (RFC 1951).
* **Wrapper**: None (no 2-byte zlib header/Adler32, no gzip header/CRC32).
* In Rust: `flate2::read::DeflateDecoder` for decompression; `flate2::write::DeflateEncoder` (level 6) for compression.

---

## 2. Binary Record & Field Structure

After decompression, the raw byte stream consists of delimited ASCII/UTF-8 records:

### 2.1 Delimiters
* `\x1a\x1a` (`0x1A 0x1A`): Record separator (begins each record).
* `\x1a` (`0x1A`): Field separator within a record.
* `\x1b` (`0x1B`): Property/value delimiter within a field.

### 2.2 Record Types

#### `Block` Record
* `Name`: Unique display name (e.g. `Humanoid Free Falling1`, `If1`).
* `Type`: Block type identifier (e.g. `HumanoidFreeFalling`, `If`, `WhileLoop3`).
* `VisualPosition`: Canvas position `X,Y` (hex numbers, e.g. `4.7D,0.2EE`).
* `ChildBlocks`: Ordered list of child block names in the primary execution flow.
* `ElseChildBlock`: Child block name for the `else` branch (or `nil` if none).
* `Inputs`: Flat list of input parameters:
  * **Mode 0 (Implicit)**: `[name, 0, value]` (type is determined implicitly by block catalog).
  * **Mode 1 (Explicit)**: `[name, 1, value, type]` (type is explicitly serialized, e.g. `String`, `Number`, `Bool`).
  * **Mode 2 (Variable)**: `[name, 2, var_name]` (references `$var_name`).
* `Outputs`: Pairs of `[output_name, bound_variable_name]`.
  * Multi-value tuple outputs are named `TUPLE_<Name>` (e.g. `TUPLE_Parameters` bound to `0`).

#### `Comment` Record
* `Name`: Comment text string.
* `Position`: Canvas position `X,Y`.
* `Size`: Canvas dimensions `W,H`.
* `Color`: Color of the comment box.

#### `Editor` Record
* `CameraPosition`: Editor camera `X,Y` coordinates.
* `CameraZoom`: Editor camera zoom factor (e.g. `0.92`).

---

## 3. Hexadecimal Number Format

Numbers in RetroStudio are serialized in hexadecimal.

### 3.1 Integer Numbers
* Standard hex integers:
  * `"3c"` / `"3C"` = 60
  * `"C2"` = 194
  * `"3E9"` = 1001

### 3.2 Fractional Numbers (3-Decimal Hex-Integer Thousandths Rule)
* Fractional numbers are stored in the format `whole_hex . frac_hex`.
* The `frac_hex` part is a **hexadecimal integer representing thousandths** (`value = whole + frac / 1000.0`):
  * `"a.1f"`: $10 + \frac{0\text{x}1\text{F}}{1000} = 10 + \frac{31}{1000} = 10.031$
  * `"0.64"`: $0 + \frac{0\text{x}64}{1000} = 0 + \frac{100}{1000} = 0.1$
  * `"0.1F4"`: $0 + \frac{0\text{x}1\text{F}4}{1000} = 0 + \frac{500}{1000} = 0.5$
  * `"0.12C"`: $0 + \frac{0\text{x}12\text{C}}{1000} = 0 + \frac{300}{1000} = 0.3$
  * `"0.2BC"`: $0 + \frac{0\text{x}2\text{BC}}{1000} = 0 + \frac{700}{1000} = 0.7$
* Negative numbers carry a leading `-`:
  * `"-a.1f"` = $-10.031$

---

## 4. Script Containers & Differences

All three script containers share the exact same binary format, header, and Base93/DEFLATE compression, but differ in permitted block environments:

| Container | Execution Context | Block Count in Fixtures | Distinguishing Block Types |
| :--- | :--- | :--- | :--- |
| **Script** | Server (Standard Script) | 278 | `RemoteFireAllClients2`, `RemoteFireClient2`, `RemoteRecieveServer2`, `KickPlayer`, `BanPlayer`, `SaveVariable`, `LoadVariable`, `SaveInstance`, `LoadInstance`, `CreateLeaderstat`, `SetLeaderstat`, `BindToClose` |
| **LocalScript** | Client (Local Player) | 277 | `RemoteFireServer2`, `RemoteRecieveClient2`, `RenderStepped`, `JumpRequest`, `InputBegan`, `InputEnded`, `TextBoxFocused`, `TextBoxFocusLost`, `GetStudioSelection`, `SetMouseBehavior` |
| **ModuleScript** | Reusable Module | 1 (minimal) | Root `ModuleReturn` returning exported value or table |

---

## 5. Connectivity & Slot Semantics

* **Roots**: Any block with `parents.is_empty()`.
  * Startup roots: standard blocks without parents run on script initialization.
  * Function definition roots: `DefineFunction` blocks act as isolated callable subtrees invoked via `ExecuteBlock` / `ExecuteFunction`.
* **Execution Flow (`ChildBlocks`)**:
  * Blocks execute sequentially along the `ChildBlocks` list.
  * For conditional blocks (`If`), the `ChildBlocks` list represents the `then` branch.
* **Else Branches (`ElseChildBlock`)**:
  * Attached exclusively to `If` blocks; points to the initial block of the `else` branch.
* **Slot Matching**:
  * Block inputs only accept values matching their declared data type in `vs_blocks.json`.
  * Outputs bind to variables referenced downstream by `$name` inputs.

---

## 6. Encoding & Round-Trip Guarantees

The encoder (`src/encoder.rs`) inverts the decoder. Given a `DecodedReport`,
`encode_records` produces the decompressed record byte stream and
`encode_payload` produces a full `.vs` payload string:
`\x1a` + zero-padded 16-digit version tag + `\x1b` + Base93(raw DEFLATE(records)).

### 6.1 Canonical Serialization Rules (empirically verified on all 556 fixture blocks)

* **Record order**: `Editor` first, then `Block` records in `block_order`, then
  `Comment` records (comment placement and field order per the V4 format gist).
* **Block records always emit all seven fields, in this exact order**:
  `Type, Name, VisualPosition, ChildBlocks, ElseChildBlock, Inputs, Outputs`.
  `ElseChildBlock` is always present; its value is the else-branch block name,
  or `nil` when there is none.
* **Editor records** emit `CameraPosition, CameraZoom`.
* **Comment records** emit `Position, Size, Color, Name` (field order from the
  V4 gist; no fixture exercises comments — the game's own serializer is the
  only authority here).
* **Field value lists**: a field with zero values is emitted as just the field
  name (no `\x1b` follows); each present value adds one `\x1b` + value.
* **Input groups**: mode 0 = `[name, 0, value]`, mode 1 = `[name, 1, value, type]`,
  mode 2 = `[name, 2, var]`. In the fixtures mode 0 groups always have 3 items
  and mode 1 groups always 4; a value of `""` is emitted as an empty item.
* **Outputs** are flattened name/value pairs.
* All strings — including hex numbers and `X,Y` positions — are passed through
  **verbatim** from the decoded report. No numeric re-formatting happens during
  round-trip; `Hex::to_hex_string` (uppercase) is only used when *creating* new
  values (e.g. by the future transpiler).
* `parents` are never serialized; they are derived from `ChildBlocks` /
  `ElseChildBlock` on decode.

### 6.2 Round-Trip Guarantees (enforced by `tests/roundtrip.rs`)

For every fixture, with `X` = original payload text:

1. **Record stream, byte-identical**: `encode_records(decode(X))` equals the
   original decompressed bytes exactly.
2. **Base93 layer, byte-identical**: `base93_encode(base93_decode(X)) == X` —
   our Base93 encoder reproduces the game's encoded text bit-for-bit, proving
   the algorithm (13/14-bit chunking, threshold 456, tail handling) matches
   the game's implementation exactly.
3. **Header + wrapper, byte-identical**: re-wrapping the game's own compressed
   bytes (`\x1a` + tag + `\x1b` + Base93) reproduces the original file byte-for-byte.
4. **Full pipeline, tree-identical**: `decode(encode(decode(X))) == decode(X)`
   (version, block order, all blocks with inputs/outputs/children/positions,
   comments, editor).
5. **Deterministic encoding**: `encode` is a pure function of the report.

### 6.3 Why Full-Payload Byte-Equality Is Impossible (documented per acceptance criteria)

The only non-reproducible layer is DEFLATE *compression*. The game compresses
with LibDeflate (Lua) level 6; we compress with `flate2`/miniz_oxide level 6.
Both produce valid raw DEFLATE streams that inflate to the identical bytes,
but the streams themselves differ:

| Fixture | Decompressed | Game (LibDeflate) | Ours (miniz lvl 6) |
| :--- | :--- | :--- | :--- |
| script.vs | 43815 B | 8305 B | 8293 B |
| local_script.vs | 45408 B | 8569 B | 8581 B |
| module_script.vs | 187 B | 142 B | 141 B |

A zlib level sweep (levels 0–9) brackets the game's sizes without ever matching
them (e.g. script.vs: zlib lvl 5 = 8433 B, lvl 6 = 8294 B, game = 8305 B), so no
mainstream compressor setting reproduces the game's byte stream. Since any valid
raw DEFLATE stream of the same bytes decodes identically (verified in tests, and
the game's `DecompressDeflate` accepts any valid stream), this is lossless in
every observable sense; byte-equality is enforced at every deterministic layer
instead.

### 6.4 Known Corners (unexercised by fixtures, covered by synthetic unit tests)

* **Mode-2 (variable) inputs** never appear in the fixtures. Their 3-item
  `[name, 2, var]` shape is symmetric with the decoder and matches the gist's
  "2 for variable" note; `encoder::tests::test_full_roundtrip_synthetic` proves
  the round trip.
* **Non-`nil` `ElseChildBlock`** never appears in the fixtures (MonikaOS has no
  if/else blocks); round-tripped in the same synthetic test.
* **Comment records** never appear in the fixtures; field order comes from the
  gist and is round-tripped synthetically.
* A hypothetical mode-1 input *without* a type item, or a mode-0 input with no
  value item, would re-serialize with an empty value item appended (tree-equal,
  not byte-equal). The game's serializer does not produce these shapes.
* Non-ASCII (latin-1) record values decode via per-byte char mapping and would
  re-encode as UTF-8, changing bytes. All fixture data is ASCII.
