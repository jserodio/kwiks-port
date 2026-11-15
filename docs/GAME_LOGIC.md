# Kwiks (1998) — Game logic specification

**Target:** `work/kwiks2.exe` (PE32 i386, Watcom C/C++32, no symbols).
**Method:** static reverse engineering with REA/Ghidra (`docs/decomp/*.c` = pseudocode) and verification via disassembly (`objdump -d -M intel`). All register semantics of the Watcom convention (EAX, EDX, EBX, ECX + stack) were verified against the assembly, not just against Ghidra's pseudocode (which mixes up the arguments in these functions).

**Legend:** **[O]** = observed directly in the binary · **[I]** = inferred (deduced with high confidence, not dynamically verified) · **[?]** = unknown / unsolved.

Naming convention: `FUN_xxxxxxxx` = procedure according to Ghidra; `LAB_xxxxxxxx` = label inside another body (Ghidra did not separate it out as a function). Addresses = VMA (image base `0x400000`).

---

## 1. State map

### 1.1 Main loop

The program does not use a timer: there is a *message pump* (unlabeled code at `0x402660`–`0x40271d`, mixed by Ghidra into `FUN_00402424`) which, when there are no messages, **calls the callback registered in `DAT_00429038`** (`call DWORD PTR ds:0x429038` at `0x4026ff`). The callback itself self-limits to 40 Hz. **[O]**

```
0x402660: init:
  FUN_004023d0()          ; DirectDraw video 640×480×16 (calls FUN_00407f7c)
  FUN_004052c0()          ; title screen + registers handler 0x4054ac
0x4026a6: pump:
  PeekMessage(...); if there is a message → Translate/Dispatch and continue
  if there is no message:
    if [DAT_00429038] == 0 → message wait (import cs:0x4621b4)
    if [DAT_00429038] != 0 && [DAT_00460fb0] == 0 → call [DAT_00429038]
```

**The state machine is the `DAT_00429038` pointer itself**: each screen is a "tick handler" that reads the keyboard with `GetAsyncKeyState` (via `FUN_0040a3a0`) and rewrites `DAT_00429038` to change screens. **[O]**

### 1.2 Screens and handlers

| Handler | Screen | Image | Transitions (observed keys) |
|---|---|---|---|
| `LAB_004054ac` | **Title** | `TITEL.JPG` (obj `0x460858`) | Enter(0x0d)/Space(0x20)/Esc(0x1b) → draws obj `0x46085c` (MENU1/EMENU1) and moves to `0x405590` · F1(0x70) → MessageBox "Knuddel's Quest" (`0x4254d9`/`0x4254e9` EN or `0x4255ee`/`0x4255fe` DE depending on flag `0x429034`) **[O]** |
| `LAB_00405590` | **Menu 1** | `MENU1/EMENU1.JPG` (obj `0x46085c`) | Enter/Space → `FUN_004043c4` (click) + `FUN_00405a70` (releases imgs) + `FUN_004045d4` (**starts game**) · Down(0x28) → draws obj `0x460860`, moves to `0x4057bc` · Esc → `FUN_00401010` (attract screen `FELD2.JPG`, handler `LAB_00401060`) **[O]** |
| `LAB_004057bc` | **Menu 2** | `MENU2/EMENU2.JPG` (obj `0x460860`) | Enter/Space → loads `DNLOGO.JPG` (obj `0x460850`, str `0x425729`) + `FUN_0040565c` (wipe of `MAKERS.PIX`/`EMAKERS.PIX` over `0x460854`) + handler `LAB_004059b4` (**credits**) · Down → draws obj `0x460864`, moves to `0x405918` · Up(0x26) → returns to `0x405590` · Esc → click + `FUN_00405a70` (callback = 0 → game sits in message wait) **[O]** (asymmetry with respect to the other menus; "quit"? **[?]**) |
| `LAB_00405918` | **Menu 3** | `MENU3/EMENU3.JPG` (obj `0x460864`) | Esc/Enter/Space → `FUN_00401010` (attract) · Up → returns to `0x4057bc` **[O]** |
| `LAB_004059b4` | **Credits** | `DNLOGO.JPG` + wipe `MAKERS.PIX`/`EMAKERS.PIX` | Esc/Enter/Space → click + `FUN_00405a70` + `FUN_00401010` **[O]** (tail of the handler not fully disassembled **[I]**) |
| `LAB_00401060` | **Attract / demo** | `FELD2.JPG` (str `0x425004`) and `FELD%d.JPG` cycle (formatted with `FUN_0040a3cb`, loaded with `FUN_004070c4`, blitted with `FUN_00409ef0`) | action → clears and changes screen **[O]** (final transition not fully traced **[?]**) |
| `LAB_00404734` | **Game** (40 Hz tick) | — | see §2. Registered by `FUN_004045d4` (`0x404720`) |
| `LAB_00404b10` | **Pause** | `PAUSED.JPG` (obj `0x460504`, loaded in `FUN_004045d4` from str `0x425458`) | P(0x50) → `FUN_004016e4` (restores game view) + dirty-rect + callback = `0x404734` **[O]** |
| `LAB_00402334` | **Game over** | `GAMEOVER.PIX` (str `0x42506c`), displayed by `FUN_004022d0` | Esc/Enter/Space → clears + `FUN_004052c0` (returns to the **title**) **[O]** |
| `LAB_00404b78` | (game cleanup: `FUN_004018f0`, `FUN_004051a8`, `FUN_0040cfe8`, Release of `0x460504`) | — | **No references found** in the traced flow **[?]** |

The complete lifecycle sequence is: **title → menu 1..3 / credits / attract → game → (pause | level complete | game over) → title**. There is no separate "credits" screen beyond the one in menu 2; the help texts are Win32 `MessageBox`es (import `cs:0x4621ac`).

### 1.3 Global keys (WM_KEYDOWN handler, region `0x40258b`–`0x4025ff`) **[O]**

| Key | VK | Effect |
|---|---|---|
| F1 | 0x70 | Help MessageBox ("Kwiks - Hilfe", str `0x4251a3`/`0x4251b1`) in the WndProc; in the game and title ticks, waits for release and end of frame |
| F2 | 0x71 | `not byte [DAT_00429064]` — "fast graphics" mode: reduces the monsters' halo to `radius/4` (min. 1) in `FUN_00403170` |
| F8 | 0x77 | Toggles music (`byte [DAT_004604d8]`): ON → `FUN_00403de0` (loads `SOUND\SPIEL.MOD`/`SPIEL.MOD` from `AUDIO.ONE`, ALoadModuleFile/APlayModule); OFF → `FUN_00403db0` (AStopModule/AFreeModuleFile/ACloseVoices) |
| F9 | 0x78 | `not byte [DAT_004604f8]` — mutes sound effects (checked by the sound wrappers) |
| F11 | 0x7a | Debug screenshot `KWDMP%3.3d.BMP` (counter `DAT_00429050`, fmt `0x4252e6`) |

---

## 2. 40 Hz game tick (`LAB_00404734`)

Registered as the callback in `DAT_00429038` at game start (`FUN_004045d4`, `0x404720`). Exact sequence (all **[O]**, disassembly `0x404734`–`0x404b74`):

```
; ---- timing ----
eax = timeGetTime()                        ; call cs:0x4620f8
if (eax - [DAT_0046080c] < 0x19 /*25 ms*/) return   ; → exactly 40 Hz
[DAT_0046080c] = timeGetTime()
[DAT_00460500]++                           ; global tick counter
if (byte [DAT_004604fc] != 0) goto MODE1   ; mode flag: 0=gameplay, 1=between levels

; ---- MODE 0: gameplay ----
FUN_00402fe4()       ; refreshes animated monster halos (bands y±(r+5))
FUN_00404cb8()       ; refreshes vertical band y±7 around the player
FUN_00403170()       ; erases/draws monster sprites (halo, radius per phase)
FUN_00404ce8()       ; HUD: %, level, score, lives + pulsating player marker
FUN_00404290([DAT_004604dc])   ; flushes the dirty-rect queue to the screen
FUN_00404f00(); FUN_00403590() ; ×3  (player and monsters are updated 3 times per tick)
pct = FUN_004021b4()           ; % = DAT_00429028*100/(640*480)
if (pct >= 0x4b /*75*/) goto LEVEL_COMPLETE

; ---- common to mode 0/1 (0x4047ee) ----
if (byte [DAT_00460508] != 0) goto EXIT   ; game-over flag
if (key(0x1b) /*Esc*/) goto EXIT
if (key(0x50) /*P*/)  → waits for release; Blt of PAUSED.JPG (FUN_0040a054);
                        [DAT_00429038] = LAB_00404b10   ; pause
if (key(0x70) /*F1*/) → waits for release and returns
return

; ---- LEVEL_COMPLETE (0x4047c0) ----
FUN_004016e4(); FUN_00403170(); FUN_00404ce8(); FUN_00408b3c()  ; paints + presents
FUN_004017cc()      ; shows full FELD%d.JPG + COMPLET.PIX overlay
FUN_00408b3c()      ; presents
byte [DAT_004604fc] = 1     ; enters mode 1
byte [DAT_00460820] = 0     ; (see the animation note below)
; falls through to the common block (0x4047ee)

; ---- MODE 1 (0x4048e2): between levels ----
if (byte [DAT_00460820] != 0):
    bouncing-logo animation: FUN_0040173c([0x460814]=vel, [0x460818]=x, [0x46081c]=y);
    FUN_00408b3c()
if no Enter(0x0d)/Esc/Space(0x20) pressed → return
waits for the 3 keys to be released
if (byte [DAT_00460820] != 0): byte [DAT_00460820] = 0; return   ; 1st press discards
; ---- NEXT_LEVEL (0x404a2a) ----
[DAT_00460810]++                    ; level index (0-based)
byte [DAT_004604fc] = byte [DAT_00460820]   ; =0 → back to gameplay mode
FUN_00401374()          ; loads FELD((level%14)+1).JPG, creates surfaces and image buffer
FUN_00404c8c()          ; player to (320, 1), no line
FUN_004028ac(level+1)   ; spawns monsters (1-based parameter)
presentations and HUD (FUN_00408e64 ×2, FUN_00408b3c, FUN_0040ce80)
FUN_00404460()          ; "los" sound (start)
[DAT_00460500] = 0; [DAT_00460814] = 2; [DAT_00460818] = 320; [DAT_0046081c] = 240
byte [DAT_00460820] = 1
vx = rand()%5 - 4; vy = rand()%5 - 4   ; both ∈ [-4,-1], retried if either is 0
FUN_004016e4(); FUN_00408b3c(); FUN_004016e4()

; ---- EXIT (0x404805): Esc during gameplay or game over ----
FUN_004018f0()          ; releases surfaces 0x42901c/0x429018/0x429020 and buffer 0x429024
FUN_004051a8()          ; HUD / high-score routine (partially unknown)
FUN_00403d34()          ; (empty function)
FUN_0040cfe8([DAT_004604dc])  ; releases the dirty-rect queue
Release([DAT_00460504]); [DAT_00460504] = 0
FUN_004022d0()          ; GAMEOVER.PIX + "tot" sound + callback = LAB_00402334 (→ title)
waits for Esc to be released
```

**Note — the bouncing-logo animation** (`0x4048f5`–`0x4049ad`): it runs in mode 1 only if `byte [DAT_00460820] != 0`. The only observed path into mode 1 (level complete) writes `DAT_00460820 = 0`, so the animation **should never be shown** in the normal flow; the state (mode=1, flag=1) it responds to has not been found to be statically reachable. It may be leftover code, or it may depend on overlapping globals (Ghidra warns "globals overlap"). **[?]** — requires dynamic verification. What is observable: the animation moves a sprite (`FUN_0040173c`) with velocity `0x460824/0x460828` ∈ [-4,-1] px/tick bouncing off the margins `[640/(2·vel), 640−640/(2·vel)]` × `[480/(2·vel), 480−480/(2·vel)]` with vel=2 (→ margins 160 px).

**Note — tick speeds:** the player and the monsters are updated **3 times per tick** (`0x404798`–`0x4047b1`), so their per-tick speeds are multiplied ×3 (see §5). There is no delta-time: everything is tied to the 40 Hz.

---

## 3. Data model (globals)

Verified Watcom convention: arguments travel in EAX, EDX, EBX, ECX (+ stack). All addresses **[O]**.

### 3.1 Surfaces and playfield

| Address | Size | Content |
|---|---|---|
| `DAT_0042901c` | ptr | **Logical playfield surface** 640×480×16 (RGB565), DDSCAPS_SYSTEMMEMORY, created in `FUN_00401374` (`0x401627`). Collision is read from it, the % is counted from it, and it is blitted to the screen |
| `DAT_00429018` | ptr | **Mirror/backup surface**, identical (same creation at `0x401633`). All playfield edits are applied to both; it serves as the source to **undo** aborted fills (`FUN_00409ef0(0x42901c ← 0x429018)`) |
| `DAT_00429020` | ptr | Temporary loading surface for the playfield JPEG |
| `DAT_00429024` | ptr | **Image buffer** (640·480 u16) with the **sanitized** playfield JPEG: 0→1, 0x1f→0x1e, 0x3e0→0x3df, 0x7f00→0x7eff, 0xffff→0xfffe (`FUN_00401374`, `0x401455`–`0x40148f`). It is the source of the fills |
| `DAT_00429028` | DWORD | **Filled-pixel counter** (basis of the percentage). Initial = `640*4 + (480-4)*4` = **4464** (the 2 px blue border) |

### 3.2 Game state

| Address | Content |
|---|---|
| `DAT_0046082c` | byte flag "drawing line" (0/1) |
| `DAT_0046082d`, `DAT_00460831` | DWORD player x, y |
| `DAT_0046083d`, `DAT_00460841` | DWORD x, y of the **line origin** (position when drawing started) |
| `DAT_00460835` | DWORD score |
| `DAT_00460839` | DWORD lives (initial **3**, `FUN_00404bc0`) |
| `DAT_00460810` | DWORD level index, 0-based |
| `DAT_00460500` | DWORD global tick counter |
| `DAT_0046080c` | DWORD timestamp of the last tick (timeGetTime) |
| `DAT_004604fc` | byte mode: 0 = gameplay, 1 = between levels |
| `DAT_00460508` | byte game-over flag (lives < 0) |
| `DAT_00460820` | byte mode-1 animation flag (see §2) |
| `DAT_00460814` | DWORD logo velocity (**2**, constant in the 3 paths that set it) |
| `DAT_00460818/81c`, `DAT_00460824/828` | x/y and vx/vy of the bouncing logo |
| `DAT_004604dc` | ptr to the **dirty-rect queue** (block of 0xfa4=4004 bytes: DWORD count + 250 RECT of 16 B; max. 250 in `FUN_00404268`); processed by `FUN_00404290` |
| `DAT_00460504` / `DAT_0046050c` | object and surface of `PAUSED.JPG` (loaded in `FUN_004045d4`) |
| `DAT_00460848`, `DAT_0046084c` | HUD object (640×22 bar) and text object (`FUN_0040d000(0x20, 0xb, 0xe)`), created in `FUN_00404bc0` |
| `DAT_00460850/854`, `DAT_00460858/85c/860/864` | credit image objects (DNLOGO/MAKERS) and menu objects (TITEL/MENU1/2/3) |
| `DAT_00460f74`, `DAT_00460f78` | playfield width and height (640, 480) |
| `DAT_00460f80` | DirectDraw object |
| `DAT_00460f8c`, `DAT_00460f90` | primary surface and destination surface for text/blits |
| `DAT_00429038` | ptr to the current **tick handler** (state machine) |
| `DAT_00429064` | byte "fast graphics" flag (F2) |
| `DAT_004604d8`, `DAT_004604f8` | bytes: music off / effects off |
| `DAT_004604e0..0x4604f4` | 6 DirectSound buffers: click (`FUN_004043c4`), monster?, gefuellt (`FUN_0040442c`), los (`FUN_00404460`), tot (`FUN_00404494`), leben (`FUN_004044c8`) |
| `DAT_00429034` | capabilities byte (set at `0x402693` with `cmp ax,7; setne`); selects `EMENU*.JPG` vs `MENU*.JPG` and the text variant |

### 3.3 Monsters (three arrays) **[O]**

| Array | Type | Stride | Counter | Fields |
|---|---|---|---|---|
| `DAT_00429068` | 1 whole "bouncer" | 0x10 | `DAT_004296a8` | +0 x, +4 y, +8 vx, +0xc vy (pixels) |
| `DAT_004296ac` | 2 "worm" | 0x658 | `DAT_0045110c` | +0 x, +4 y, +8 vx, +0xc vy; +0x10 current trail length; +0x14 maximum length; +0x18 trail_x[]; +0x938 trail_y[] |
| `DAT_00451110` | 3 subpixel "bouncer" | 0x18 | `DAT_00451a70` | +0 X, +4 Y (**fixed point 1/8 px**), +8 vx, +0xc vy (1/8 px units) |

---

## 4. Collision and filling (QIX core)

### 4.1 Playfield representation

The playfield **is the `DAT_0042901c` surface itself** (640×480 matrix of u16, RGB565) plus a parallel image buffer at `DAT_00429024`. There is no cell grid or bit bitmap: collision is a **pixel query**. Reserved values (deduced from the sanitization in `FUN_00401374` and the values compared in the code) **[O]**:

| Value | Meaning |
|---|---|
| `0x0000` | free (not filled) — black |
| `0xffff` | white: **own line in progress** |
| `0x001f` | blue: **closed line / border** |
| `0x7f00` | monster marker type 1 and 3 |
| `0x03e0` | monster marker type 2 (worm) |
| anything else | already-filled image pixel (0→1, 0x1f→0x1e, 0x3e0→0x3df, 0x7f00→0x7eff, 0xffff→0xfffe so as not to collide with the reserved ones) |

The inner playable area starts as black with a 2 px blue border (drawn in `FUN_00401374`: fill `(0,0)-(640,480)` blue and `(2,2)-(637,477)` black with `FUN_0040a510`). The playfield image only appears once filled.

### 4.2 Primitives

- **`FUN_00401960(surf=EAX, x=EDX, y=EBX) → u16`** (`0x401960`): Locks the surface (vtable +0x64, retry with Restore +0x6c if `DDERR_SURFACELOST 0x887601C2`), reads `*(u16*)(lpSurface + y*lPitch + x*2)`, Unlock (+0x80). **[O]**
- **`FUN_0040a510(surf=EAX, x1=EDX, y1=EBX, x2=ECX, y2=stack, color=stack, flag=stack)`** (`0x40a510`): builds a `RECT` with `SetRect(x1,y1,x2,y2)` (import `cs:0x4621c8`) and performs a `Blt` with `DDBLT_COLORFILL|DDBLT_WAIT` (0x1000400). **Win32-style rect semantics (right/bottom exclusive)** → `(x, y, x+1, y+1)` paints **1 pixel**. Used for everything: player/monster markers, borders and fill spans. Flag=0 takes another path (line) with no relevant in-game use. **[O]**
- **`FUN_004019c4(surf=EAX, x=EDX, y=EBX, match=ECX, new=stack)`** (`0x4019c4`): classic **span flood fill** over the pixels == `match`, painting them with `new` via `FUN_0040a510`. Seed stack of `(x, y±1)` with a maximum of **0x1f5 (500)** entries; traversal: walks left while there is `match`, fills the span to the right pixel by pixel, seeds above/below at the transitions. If `new == 0x1f` **and** `surf == DAT_0042901c`, it adds the number of painted pixels to counter `DAT_00429028` (`0x401b80`–`0x401b99`). **[O]**
- **`FUN_00401c8c(surf=EAX, x=EBX, y=ECX, imgbuf=EDX) → al`** (`0x401c8c`): **conquest fill**. Only acts if the target pixel is 0. Span flood fill (stack of double mask/image pointers, max. 500) that writes the corresponding pixel from the image buffer into the mask (`*psVar7 = *psVar4`, with defensive 0→1), in parallel and with stride 1280 for the image. **If it finds `0x7f00` or `0x3e0` (monster) it aborts and returns 0** (`0x401da2`–`0x401dcb`) — with the partial fill already applied. If it finishes, it returns 1 and, only if `surf == DAT_0042901c` (`0x401efa`): adds the pixels `P` to counter `DAT_00429028` and calls `FUN_0040526c` with the points (see §6). **[O]**
- **`FUN_00401bac(x=EAX, y=EDX)`**: flood fill `FUN_004019c4` white(0xffff)→blue(0x1f) from 4 axial seeds of (x,y), on **both** surfaces (`0x42901c` and `0x429018`). Converts the line in progress into a closed line. **[O]**
- **`FUN_004021e4(x=EAX, y=EDX)`**: same but white→black(0): **erases the line** (death or cancellation). **[O]**
- **`FUN_00409ef0(dst=EAX, src=EDX)`**: full-screen `Blt` with retry (`DDERR_SURFACELOST`/`WASSTILLDRAWING`). `FUN_00409ef0(0x42901c, 0x429018)` = **restore/undo**; `FUN_004016e4(dst)` = `FUN_00409ef0(dst, 0x42901c)` = blit the playfield. **[O]**
- **`FUN_004021b4() → int`**: percentage = `DAT_00429028 * 100 / (DAT_00460f74 * DAT_00460f78)` (integers). **[O]**

### 4.3 Closing an area (complete flow)

When the player moves while in "drawing" mode and the target pixel is **claimed** (≠ 0, ≠ 0xffff, ≠ 0x7f00, ≠ 0x3e0) → the line has touched already-conquered territory and **closes** (in `FUN_00404f00`, block `0x405123`–`0x405198`) **[O]**:

1. `FUN_0040442c()` — "gefuellt" sound.
2. `FUN_00401bac(player_x, player_y)` — the entire connected white line → blue (on both surfaces). Its pixels are counted into `DAT_00429028`.
3. `FUN_00401f50(px, py, lsx, lsy)` with `(px,py)` = player and `(lsx,lsy)` = line origin (`DAT_0046083d/841`). Seeds `FUN_00401c8c` at **8 points**: the **4 diagonals** of the player `(px±1, py±1)` and the **4 axial neighbors** of the origin `(lsx±1, lsy)`, `(lsx, lsy±1)` (`0x401f60`–`0x4021ad`). For each seed:
   - `r = FUN_00401c8c(0x42901c, imgbuf, x, y)`:
     - `r == 1` (region without monsters or already filled) → `FUN_00401c8c(0x429018, …)` repeats the fill on the mirror (keeping it in sync).
     - `r == 0` (the region contained a monster) → `FUN_00409ef0(0x42901c ← 0x429018)`: the partial fill is **undone**. The region is not conquered.
4. `FUN_004016e4()` (blit to screen), `FUN_00404268` (full dirty-rect), `DAT_0046082c = 0`.

**Mechanism of "a region with a monster is not conquered" [O/I]:** the fill only traverses free pixels (0) and stops at any other value, so lines, the border and already-filled pixels act as walls. Each seed explores the region connected to its position; if a monster marker is found during the traversal, that particular region is left unconquered (the partial one is undone with the mirror). The exact topology of which seeds cover the inside vs. the outside of the loop (the 4 diagonals of one end + the 4 axial neighbors of the other) **has not been dynamically verified**; the "conquer what has no monsters" semantics is what is observed. **[I]**

**Precision note:** the initial border paints `(2,2)-(637,477)` with exclusive semantics → 635×475 black px, while the initial counter assumes 636×476 (4464). There is a 1 px discrepancy at the right/bottom border and of ~1100 px in the counter relative to the real pixel count; the counter behavior (formula) is the observed one. **[O]**

### 4.4 Death

- In `FUN_00404f00` (player while drawing steps on `0xffff`/`0x7f00`/`0x3e0`) and in `FUN_004051d8` (monster touches the line, called from `FUN_00403590`): sound `FUN_00404494` ("tot"), **lives--** (`DAT_00460839`); if `lives < 0` → `DAT_00460508 = 1` (game over: the tick exits to `FUN_004022d0`); otherwise → `FUN_004021e4` (erases the white line), player to **(width/2, 1)** = (320, 1), `DAT_0046082c = 0`, full refresh. **[O]**

---

## 5. Physics

### 5.1 Player (`FUN_00404f00`, 3 calls/tick)

- **Speed:** 1 px per call → **up to 3 px/tick = 120 px/s** (with a key held). **[O]**
- Keys (VK from `GetAsyncKeyState`): 0x27 right (x++), 0x28 down (y++), 0x25 left (x--), 0x26 up (y--). Clamp: `1 ≤ x ≤ width-2`, `1 ≤ y ≤ height-2`. A single key per call (priority right > down > left > up). **[O]**
- **Movement model:** the pixel on the surface at the **new** position is read (`FUN_00401960`):
  - `0` (free) → if not drawing, saves the line origin (`0x46083d/841`) and sets `DAT_0046082c = 1`; draws its white pixel (1×1 with `FUN_0040a510(…, 0xffff, 1)` on **both** surfaces).
  - `0xffff`/`0x7f00`/`0x3e0` while drawing and having moved → **death** (§4.4).
  - Any other value (claimed zone) while drawing and having moved → **closure** (§4.3). If it did not move or was not drawing, it simply walks over the claimed zone.
- Initial and respawn position: (width/2, 1) = (320, 1), on the border. **[O]**
- The player marker is a 1-pixel white dot at (x,y) with a pulsating halo `(x±i, y±i)` with `i = (tick % 16)/5` in `FUN_00404ce8`. **[O]**

### 5.2 Monsters (`FUN_00403590`, 3 calls/tick; spawns in `FUN_004028ac(level)`)

Generation per level (1-based parameter `n`) **[O]**:

| Type | Count | Initial position | Initial speed |
|---|---|---|---|
| 1 (whole bouncer) | `n - n/3` | `x = rand()%(640-4)+2`, `y = rand()%(480-4)+2` | `vx, vy = rand()%3-1` retrying until both ≠ 0 → always **±1 px/tick diagonal** |
| 2 (worm) | `n/2` | same | `vx = rand()%3-1`; if vx≠0 → vy=0, if vx=0 → vy=rand()%3-1 (axis-aligned) |
| 3 (subpixel bouncer) | `n/3` | `X = (rand()%(640-4))*8 + 16`, `Y = (rand()%(480-4))*8 + 16` (1/8 px) | `vx, vy = rand()%0x11 - 8` ∈ [-8,8], both ≠ 0 → **0.125..1 px/tick per axis** |

Movement (common to types 1 and 3; type 3 uses coordinates/8 rounded to integer) **[O]**:

```
pixel = FUN_00401960(0x42901c, x+vx, y+vy)
if (pixel == 0)        { x += vx; y += vy }              ; free → advances
else if (pixel == 0xffff) FUN_004051d8()                  ; touches the line → kills the player
else { if (FUN_00401960(x+vx, y) == 0) vy = -vy          ; bounce: try the X axis
       else vx = -vx }
```

Its previous pixel is erased (1×1 black fill) and the marker `0x7f00` (types 1 and 3) or `0x3e0` (type 2) is drawn. Effective: up to 3 px/tick (type 1) or 3/8 px/tick (type 3). **[O]**

Worm (type 2) **[O]**:
- Carries a **trail** of positions (arrays `+0x18`/`+0x938`, max. `(level*10) % 200`); when advancing it shifts the trail and erases the tail end once the maximum length is reached.
- Movement probability: it only advances if the destination is free **and** `rand()%50 != 0`; with `rand()%50 == 0` it **turns** (switches to the perpendicular axis: if vy==0 → vy=rand()%3-1 and vx=0; otherwise → vx=rand()%3-1 and vy=0). If the destination is `0xffff` (and rand≠0) → `FUN_004051d8` (kills the player).
- **If the 4 axial cells around its head are all occupied** (≠ 0) → the worm is **trapped**: it erases itself and its entire trail and refreshes the area (`FUN_004016f8`). This is the removal of monsters in closed areas. **[O]**

Graphics effects (do not affect physics) **[O]**: animated halo around each monster with radius `((tick + index) % 20)*5 + 1` (divided by 4 with F2); `FUN_00402fe4` refreshes the band `y±(r+5)` and `FUN_00403170`/`FUN_00402ae4` redraw the sprite (the 3 100×100 sprites loaded by `FUN_00402720` to `0x456894/0x451a74/0x45b6b4`).

---

## 6. Scoring

When a region is conquered (`FUN_00401c8c` succeeds, only on `DAT_0042901c`) **[O]** (`0x401ef3`–`0x401f3c`):

```
P = pixels filled by the fill
DAT_00429028 += P
points = (DAT_00460810 + 1) * (P * 1000 / (640*480))     ; integer division
FUN_0040526c(points)
```

That is: **points = level (1-based) × (thousandths of screen conquered)**. E.g.: filling 1% of the playfield (3072 px) gives 10·level points.

`FUN_0040526c(points)` (`0x40526c`): `score (DAT_00460835) += points`; if the value crosses a multiple of **5000** upward → **+1 life** (`DAT_00460839++`) and sound `FUN_004044c8` ("leben"). Jumping over several multiples at once grants a single life. **[O]**

- Game start: score = 0, lives = **3** (`FUN_00404bc0`). **[O]**
- The white→blue line fill (`FUN_004019c4`) **does add pixels to the percentage but grants no points** (it does not call `FUN_0040526c`). **[O]**
- HUD (`FUN_00404ce8`, 22 px top bar): four texts with formats `%3.3d`, `%3.3d`, `%6.6d`, `%3.3d` in this order = (percentage, level, score, lives) — the order of the calls is observed; the exact value-format association is **[I]**.
- Level threshold: `pct >= 75` (`cmp eax, 0x4b` at `0x4047bb`). **[O]**

---

## 7. Key functions

| Address | Responsibility |
|---|---|
| `0x402660` (LAB) | Main loop / message pump; calls the `DAT_00429038` callback |
| `0x4023d0` | Video init: `FUN_00407f7c(0x10, 1, 640, 480, 640, 480, hInst)` → 640×480×16 fullscreen |
| `0x40258b` (LAB) | Global key handler F2/F8/F9/F11 inside the WndProc |
| `0x404734` (LAB) | **40 Hz game tick** (see §2) |
| `0x404b10` (LAB) | Pause tick (P toggles) |
| `0x4054ac/0x405590/0x4057bc/0x405918/0x4059b4` (LABs) | Title and menus 1-3 / credits |
| `0x401060` (LAB) | Attract screen (`FELD%d.JPG` cycle) |
| `0x402334` (LAB) | Game-over screen → title |
| `0x4045d4` | **Game start**: loads `PAUSED.JPG`, sprites, dirty-rect queue, `FUN_00401374`, level-1 spawn, installs tick `0x404734`, music |
| `0x404a2a` (LAB, in the tick) | **Next level**: reloads playfield, spawn, resets counters |
| `0x401374` | **Playfield loading** `FELD((level%14)+1).JPG`: creates sanitized image buffer (`DAT_00429024`) and surfaces `DAT_0042901c/0x429018` with blue border; initializes `DAT_00429028 = 4464` |
| `0x4028ac` | **Monster generation** per level (3 types, see §5.2) |
| `0x404f00` | **Player**: input, 1 px movement, line drawing, closure, death |
| `0x403590` | **Monsters**: erase/draw, movement, bounces, worm trail, player death |
| `0x401960` | Pixel query (collision) |
| `0x40a510` | 1 px..n rectangle fill (color fill with `Blt`) |
| `0x4019c4` | Span flood fill (white→blue / white→black) with counter |
| `0x401c8c` | **Conquest fill** copying the image; aborts on monsters; scores |
| `0x401f50` | Seeds the 8 fills on line closure + undo with mirror |
| `0x401bac` / `0x4021e4` | White line → blue / → black (both surfaces) |
| `0x4021b4` | Fill percentage |
| `0x40526c` | Scoring + extra life every 5000 |
| `0x4051d8` | Player death by monster (shares logic with `FUN_00404f00`) |
| `0x404c8c` | Player reset to (320, 1) |
| `0x404bc0` | Session init: HUD, lives=3, score=0 |
| `0x404ce8` | HUD (%, level, score, lives) + player marker |
| `0x402fe4` / `0x403170` / `0x402ae4` | Animated monster halos (refresh, draw, sprite) |
| `0x404268` / `0x404290` | Dirty-rect queue (enqueue / flush to screen) |
| `0x4016e4` / `0x4016f8` | Blit the playfield to screen (full / per rectangle) |
| `0x409ef0` / `0x409f48` | Full / per-rectangle `Blt` between surfaces (restore = mirror→logical) |
| `0x4017cc` | Level-complete screen (`FELD%d.JPG` + `COMPLET.PIX`) |
| `0x4022d0` | Game over: `GAMEOVER.PIX` + sound + move to title (`FUN_004052c0`) |
| `0x4052c0` | Title screen (`TITEL.JPG`, loads menu images) |
| `0x405a70` | Menu image release (callback = 0) |
| `0x401010` | Static attract screen (`FELD2.JPG`, handler `0x401060`) |
| `0x40565c` | Wipe effect (2 px wipe every 2 ms) for credits |
| `0x40a3a0` | `GetAsyncKeyState(vk) < 0` |
| `0x40beb2` | `rand()` LCG (seed `s*0x41C64E6D + 0x3039`, returns `(s>>16)&0x7fff`) |
| `0x40a3cb` | `sprintf` (HUD formats and `FELD%d.JPG`) |
| `0x40d098` / `0x40d000` | Text drawing / text object creation |
| `0x4043c4/0x40442c/0x404460/0x404494/0x4044c8` | DirectSound wrappers (click / gefuellt / los / tot / leben); mute with `DAT_004604f8` |
| `0x403de0` / `0x403db0` | Music: load+play (`SOUND\SPIEL.MOD` from `AUDIO.ONE`) / stop |
| `0x4070c4` / `0x40a419` / `0x40b1d4` | Load JPEG→surface / buffer allocation / memory allocation |
| `0x4018f0` | Releases surfaces and buffer on game exit |
| `0x402720` | Loads the 3 100×100 monster sprites |

---

## 8. Open issues

1. **Activation of the bouncing-logo animation** in mode 1 (§2): the state that triggers it has not been found statically. **[?]**
2. **Exact topology of the 8 seeds of `FUN_00401f50`** (which side of the loop each one conquers). The global semantics are verified; the fine geometry requires dynamic testing. **[I]**
3. **`FUN_004051a8` / `DAT_00460848/84c`**: high-score routine / HUD objects with vtable; not analyzed in depth. **[?]**
4. **`LAB_00404b78`**: handler with no references found (dead code?). **[?]**
5. **`FUN_00402ae4`** (halo drawing): the internal sprite rasterization algorithm has not been dissected (does not affect logic). **[?]**
6. **1 px border discrepancy** between the initial fill and the counter (§4.3). **[O unsolved]**
7. The "4th monster type" suggested in earlier analyses **is not confirmed**: there are 3 types with 3 100×100 sprites. The MOD music and the `.PIX` format are out of the scope of this document.

---
_Summary in 8 lines:_
1. State machine = pointer `DAT_00429038` invoked by the message pump (`0x4026ff`): title `0x4054ac`, menus `0x405590/0x4057bc/0x405918`, credits `0x4059b4`, attract `0x401060`, game `0x404734`, pause `0x404b10`, game over `0x402334`.
2. 40 Hz tick (`0x404734`, Δ<25 ms skips the frame): halos → HUD → dirty-rects → (player+monsters)×3 → percentage ≥75 % ⇒ COMPLET; Esc/game over ⇒ exit.
3. Data: playfield = u16 640×480 surface (`DAT_0042901c`) + mirror (`0x429018`) + sanitized image buffer (`0x429024`); fill counter `DAT_00429028`; player `0x46082d/831`, lives `0x460839`, score `0x460835`, level `0x460810`.
4. Collision = pixel query (`FUN_00401960`): 0 free, 0xffff line, 0x1f closed, 0x7f00/0x3e0 monsters; anything else = image.
5. Fill = span flood fill (`FUN_00401c8c`) copying the image from `0x429024`; if it touches a monster marker it is undone with the mirror (`FUN_00409ef0`); closure triggers 8 seeds from the line endpoints (`FUN_00401f50`).
6. Physics: player 1 px/call ×3 (120 px/s), monsters: bouncers ±1 px diag., worms ±1 px axial with trail `(level*10)%200` and random 1/50 turn, subpixel ±1/8..1 px; bounce by component inversion.
7. Points = (level+1)×(P×1000/307200) per conquered region; extra life on crossing every 5000; 3 initial lives; level threshold 75 %.
8. Estimated coverage of the documented game logic: **~85 %** (remaining: logo animation, fine seed geometry, `FUN_004051a8`, halo rasterization and the 1 px border offset).
