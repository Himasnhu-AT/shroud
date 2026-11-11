# Reddit plan and post drafts: {GAME} (working title Shroud)

Drafted 11 Nov 2026. Drafting only: nothing posted, nobody contacted.
Binding inputs: `company-context/founder/gtm-and-fundraise-2026-11.md` and the research brief (two accounts, max one self-promo post per sub per week, reply within 2 h, no paid ads, no "CS2-like" in titles). Targets: 300 stranger downloads by 15 Dec 2026; 500 wishlists before Next Fest (registration closes 10 Jan 2027, fest 22 Feb - 1 Mar 2027).

**Placeholders.** `{GAME}` everywhere. "Shroud" is a famous FPS streamer's name: rename before the first player-sub post (week 2). Rule sources in the research brief are third-party mirrors, so read each sidebar on the day you post.

**Accounts.** Studio = S (player subs). Personal = P (r/rust, r/bevy, r/rust_gamedev, r/MachineLearning). S and P never vote on, comment on or crosspost each other's threads.

**Copy rule.** Drafts describe the actual method (player model, genetic algorithm, NEAT, bandit tactics, trained on a ghost of you) and make no claim about how assets were made. Drafts don't mention AI coding agents (founder doc: investors only).

---

## 1. Calendar (post window: 8-10 am ET = 6:30-8:30 pm IST, Tue-Thu)

**Week 0 (11-15 Nov): prep.**
- Verify both accounts' email; no VPN.
- Comment daily, about 9 genuine comments per planned post. S: r/DestroyMyGame (give real critiques of others' clips, the natural fit), r/playmygame (play and review two games a week), r/IndieGaming, r/macgaming, r/tacticalshooters. P: r/rust, r/bevy, r/rust_gamedev (answer questions, no self-links).
- Read each target sidebar; message the r/rust mods about a commercial-game write-up.
- Capture clips A-C (section 2). Settle the name, the generative-assets question (section 5), and the itch telemetry wording.

| Week | Day (window) | Sub | Acct | Format | Goal |
|---|---|---|---|---|---|
| 1 (16-22 Nov) | Tue 17 | r/DestroyMyGame | S | 30 s video (clip A), details in comments | Feedback (first 30 s, readability). Works under the working title |
| 2 (23-29 Nov) | Tue 24 | r/playmygame | S | GIF + description + itch link, flair after submit | Downloads + feedback. Needs the new name |
| 2 | Wed 25 | r/rust_gamedev | P | Text write-up, one diagram, no store link | Credibility, feedback on method. Not downloads |
| 3 (30 Nov - 6 Dec) | Tue 1 Dec | r/IndieGaming | S | GIF (clip B), itch link in first comment | Downloads |
| 3 | Wed 2 | r/bevy | P | Engine/perf write-up + screenshots | Credibility |
| 3 | Thu 3 | r/DestroyMyGame | S | Bodycam-mode variant (clip C), only if week-1 feedback wasn't "unreadable" | Feedback on look |
| 4 (7-13 Dec) | Tue 8 | r/macgaming | S | GIF + short text, only if native Mac performance is measured | Downloads (Mac) |
| 4 | Wed 9 | r/MachineLearning | P | `[P]` post, or the weekly self-promo thread if the sidebar says so | Credibility (investors) |
| 4 | Thu 10 | r/IndieDev | S | "What N strangers did in my shooter", real numbers only | Feedback + downloads |
| 5 (14-20 Dec) | Tue 15 | r/tacticalshooters or r/FPS | S | Clip of the lamp being shot out or the flank (clip D), itch link, or Steam link if the page is live | Downloads; wishlists if Steam is live |

Notes: avoid Thu 26 Nov (US Thanksgiving, thin traffic). If the rename slips past 24 Nov, hold every S post except r/DestroyMyGame. From week 5, once the Steam page is live (founder plan: 16 Dec - 10 Jan), switch S links to the Steam page (goal: wishlists) and keep itch as the free build. Don't post the same clip twice in a week; rewrite each post.

---

## 2. Clips to capture (owner)

Capture with `crates/game/src/dev/cinema.rs` (capture + camera path) or `scripts/play.py` offscreen, or screen-record real play. Use real play from one save, no staged or faked callouts. If a clip is cut together from two matches, say so in the first comment. Avoid the Depot map (its layout is being reworked as too close to a well-known map).

- **Clip A (30 s, r/DestroyMyGame).** Round 1: you hold a doorway on Foundry or Sirocco and win. Hard cut to a title card "Two matches later". The squad now jiggle-peeks your angle and flashes it. Show the radio line and the debrief line ("They noticed you hold angles; they started jiggle-peeking and flashing."). First 3 s must show action, not a menu.
- **Clip B (8-12 s GIF, r/IndieGaming).** You reload at half a mag behind cover (twice earlier in the match). The radio shows "He reloads early - push on the mag", a bot sprints you. Radio text must be readable in the GIF.
- **Clip C (30 s, bodycam).** The same moment as clip A with the optional bodycam camera on.
- **Clip D (10 s GIF).** A lamp over your camping spot is shot out by a member standing out of your sight, then the flash or push. Log line in game: "Shooting out his light".
- **Diagram (r/rust_gamedev).** One image: player model -> ghost of you -> background sim (GA + NEAT + bandit) -> next match's squad, with a box "fixed by difficulty: reaction, aim error, damage".
- **Debrief screenshot** with the "what they learned" line, for the first comments.

---

## 3. Drafts

### 3.1 r/DestroyMyGame (S, video post)
**Title:** I played it twice. The second time, the bots knew exactly where I like to hold.
**Body:** none (video only). Clip A.
**First comment:**
> Dev here. {GAME} is a single-player tactical defusal shooter (first to 3 rounds, buy menu, grenades) where the enemy squad studies how you play between matches. It doesn't get faster or more accurate; it gets smarter about where it stands and how it peeks.
> Destroy it, please: (1) did the first 30 seconds tell you what the game is? (2) could you read the radio calls? (3) does the look feel cheap or fine? Free build on itch if you want to try it: {ITCH_URL_UTM}
> {Cut note if applicable.}

### 3.2 r/playmygame (S)
**Title:** [Windows/Mac] Free build: a bomb-defusal shooter where the enemy squad learns your habits ({GAME})
**Body:**
> I'm a solo dev. {GAME} is a first-person shooter: you attack, a squad defends two sites, first to 3 rounds, with a round economy, buy menu and grenades. Between matches the squad trains on a ghost of you and comes back with counters to what it saw: hold an angle and they jiggle-peek and flash, reload early and someone rushes the sound of your mag.
> Fixed by difficulty, never by evolution: their reaction time, aim error and damage. They get smarter, not faster.
> **Play it:** {ITCH_URL_UTM} (free, Windows and macOS, about {N} min per match)
> **What I want to know:** 1) Is Recruit fair? An early tester found the default too hard. 2) Did you notice the squad change between matches? When? 3) Where did you quit, and why? 4) Crashes or slow frames: tell me your hardware.
> The build records anonymous play stats ({OWNER TO CONFIRM exact wording}). Comments here or on the itch page both reach me.
**Flair:** set after submit (Windows/Mac). **GIF:** clip B.
**First comment:** the debrief screenshot plus "this is what the squad says it learned after a match".

### 3.3 r/IndieGaming (S)
**Title:** The squad radioed "He reloads early - push on the mag." Then they did.
**Media:** clip B (native upload).
**First comment:**
> Dev here. The squad keeps a model of how you play (where you camp, when you reload, which way you strafe) and trains between matches. Their aim and reaction never improve; only their tactics do. Free build for Windows and Mac: {ITCH_URL_UTM}. Happy to answer anything about how it works.

### 3.4 r/rust_gamedev (P; adapt to r/rust only after mods agree)
**Title:** How the enemy squad in my Rust FPS learns your habits (player model -> ghost -> GA, NEAT and bandit tactics), with fixed aim and reaction limits
**Body:**
> I've been building a single-player shooter in Rust (Bevy 0.20, avian3d) where the enemy squad changes between matches based on how *you* play. Write-up of the design, including what didn't work.
> **Pipeline**
> 1. *Player model.* Per-match telemetry: position heatmap, fight ranges, strafe bias, plant-site preference, plus a live style profile (angle holder, entry fragger, early reloader, precision aimer) with hysteresis.
> 2. *Ghost.* A 2D stand-in for you built from the model. It is an approximation (for example, it doesn't aim down sights), and that gap is the main weakness.
> 3. *Background training.* A fast 2D sim of the same arenas runs while you play or sit in menus. Trait genome: genetic algorithm, population 24, tournament selection (k=3), uniform crossover, gaussian mutation, 2 elites, 5 deployed per match. Combat micro (strafe, peek, fire gate): NEAT, 14 inputs and 4 outputs.
> 4. *Tactics.* A contextual bandit (LinUCB) picks between Crossfire, Pin-and-flank, Trade and Bait, with hand-tuned weights as a prior that fades with data. A utility layer handles goals; a doctrine layer picks counters for your detected habits.
> **Fairness.** Reaction time, aim error and damage per hit are limits set by difficulty and are inert in the genome. A generation-20 squad shoots exactly like generation 1; it only plays better. In my test suite (sim, Recruit mechanics) evolved squads beat fresh ones {54% -> 77% round wins: RE-VERIFY against `crates/brain/tests/brain.rs` before posting}. That is a sim number, not a player-study result.
> **The design rule.** The first version let reaction time and aim error loosen a little each generation. I removed that: mechanics are now fixed by difficulty, and only technique genes evolve. A fixed-mechanics test still shows the evolved squad beating fresh squads (54% → 77% round wins), so the gain comes from play, not faster trigger fingers.
> This is a commercial project with a free build; mods, if links aren't OK here I'll keep it out. Happy to go deeper on any layer.
**Media:** the diagram. **First comment:** a short list of crates (`brain` is pure Rust, no Bevy) and the test names for the claims; itch link only if the sidebar allows or someone asks.

### 3.5 r/bevy (P)
**Title:** Shipping a Bevy 0.20 FPS on laptops: the performance rules that mattered (merged level meshes, nearest-K lamp shadows, pipeline warm-up)
**Body:**
> {GAME} is a first-person shooter on Bevy 0.20.1 and avian3d 0.8. What moved frame time for us:
> - Plain level blocks are merged per material into a few meshes.
> - Static scenery vs pooled, moving FX are separate (`Scenery` / `Actor`), so culling and batching treat them differently.
> - Lamp shadows only for the nearest K lights, with K driven by a render-budget governor.
> - Pipelines are warmed at round start so the first grenade doesn't hitch.
> - Textures cooked to KTX2 offline.
> Numbers: {MEASURED fps/frame-time per machine, before and after each change; do not post without measurements}.
> The enemy AI lives in a pure-Rust crate with no Bevy dependency so the trainer can run it headless at high speed; the game and the sim share one arena description. Questions welcome.
**Media:** 2-3 before/after frame-time screenshots. **First comment:** the one thing that surprised you most (owner's real answer).

### 3.6 r/MachineLearning (P, `[P]` or the self-promo thread)
**Title:** [P] Evolving a shooter squad with GA, NEAT and contextual bandits against a ghost of the player, with fixed mechanical limits
**Body:** the 3.4 body trimmed to the method: ghost fidelity, fitness, the fixed-limit design, 3 honest limitations (sim-to-game gap, small populations, no player study yet), no store link. Ask: "has anyone compared bandits against evolved tactics for opponent modelling?" Check the sidebar first: if promotion is only allowed in the weekly thread, post a 4-line version there.
**Optional extra line (use only once the owner confirms no shipped asset is generative):** "No LLMs or generative models are used in the game, in code, art or audio." Without that confirmation, use only: "The enemy behaviour is classic evolutionary computation and bandits, no neural language or image models."

---

## 4. Reply playbook

Reply within 2 h. Short, concrete, never combative. Replies to a downvoted thread stay polite.

1. **"Isn't this just CS?"** Yes, the round and economy loop is CS-style on purpose. What's new is a single-player squad that studies you between matches and counters your habits.
2. **"AI slop?"** The squad uses a genetic algorithm, NEAT neural nets and a bandit, trained on your own matches against a ghost of you. It isn't a language or image model. {Add the optional line only once owner-confirmed. If asked about art, audio or voice, answer from the confirmed asset list, not from memory.}
3. **"Is it cheating / an aimbot?"** No. Reaction time, aim error and damage are fixed by difficulty and never change with generation. Gen 20 shoots like gen 1; it positions, flanks and times better. They also have no wallhack: they only know what a squadmate saw, heard or felt.
4. **"Mac performance?"** {Only after measuring: "Runs at X fps on [chip] at [settings]."} Until then: "Honestly unmeasured on base chips. Tell me your chip and fps and I'll fix the worst cases."
5. **"Price?"** The build is free on itch now. Pricing for any Steam release isn't decided.
6. **"Steam link?"** Not yet. {After the page is live: link and "wishlist helps".}
7. **"Name's the streamer!"** Yes, it's a working title and it's changing. {Edit once renamed.}
8. **"Rust/Bevy: why, and how?"** Bevy 0.20 and avian3d 0.8. Rust because the trainer needs to run the AI fast and headless. AI is a separate crate with no Bevy dependency. Happy to go deeper in {r/bevy thread}.
9. **"Too hard" / "too easy."** Try Recruit and tell me which fight you lost and how: that's the most useful feedback I can get.
10. **"Unrecord ripoff" (bodycam).** It's an optional camera mode, not the point of the game. The art is simple and the focus is the squad AI.
11. **"Does it really learn, or is it scripted?"** Between matches it trains in a background sim against a ghost of you, and picks counters for habits it detected. The ghost is a simplified stand-in, so it isn't mind reading, and the first matches are generic.

---

## 5. Tracking

**Links.** `?utm_source=reddit&utm_medium=social&utm_campaign=shroud_nov26&utm_content=<sub>_<post#>` (example `dmg_1`, `pmg_1`). itch's dashboard shows referrer domains but, as far as I know, not per-post UTM breakdown ({VERIFY}). To split by post, use per-post paths on a studio-owned domain that redirect to itch (`/go/pmg1`) and count hits. Steam's traffic reports do support UTM: use the same scheme once the page is live.

**Log per post** (sheet): date/time (ET and IST), sub, account, title, clip, flair, upvotes and ratio at 2 h/24 h/72 h, comments, replies given within 2 h (Y/N), itch views, downloads, download rate, any removal and its stated reason, themes of top 3 comments, Steam wishlists (after page live).

**Signals (my proposed thresholds, adjust once you see real numbers).**
- Pace for 300 by 15 Dec: about 60 stranger downloads by 29 Nov, 150 by 6 Dec, 250 by 13 Dec.
- Download rate (downloads / itch views) at least 10% (founder target).
- A post with fewer than 10 downloads (or fewer than 5 non-friend comments) after 48 h: don't repost in that sub; change the clip or the angle first.
- A post with 30+ downloads or 20+ substantive comments: continue that sub and reuse its format.
- Two removals by one sub's mods, or a mod warning: stop that sub, ask the mods what they'd accept, don't appeal publicly.
- Wishlists (after the Steam page is live): need about 500 by 22 Feb; check weekly, and if under about 25 per week through January, fix the page and clips before adding Reddit volume. No source I found gives a Reddit-to-wishlist rate for an indie shooter.
- Never buy ads, votes or awards; no cross-boosting between accounts.

---

## 6. Blocked on owner

1. **Final name** (before week 2 S posts).
2. **Generative assets?** Any shipped art, voice lines (including squad radio) or text. Decides the optional line, Steam disclosure and reply 2.
3. **Capture clips A-D**, the diagram and the debrief screenshot; confirm the radio lines shown are what the build prints.
4. **Telemetry disclosure wording** for the itch page and the playmygame post.
5. **itch URL, studio account name, redirect domain**; Steam page date.
6. **Mac:** is the .dmg native Apple Silicon, and measured fps on a base chip? (Gates r/macgaming and reply 4.)
7. **Technical claims to re-verify:** 54% -> 77% sim win rates, and exact limit values.
8. **Bevy performance numbers** for r/bevy.
9. **Engine wording:** is the engine layer called Hull, and is Shroud's code public?
10. **Price intent** for reply 5.
11. **r/rust mods' answer** on commercial write-ups; read every sidebar on the day.
