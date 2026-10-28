# What people who record screen demos ask for

This is a **qualitative desk sample**, not a survey. It was gathered on 28 October 2026 (IST) by reading public threads and issue trackers. Nobody was interviewed and no one was paid. Counts below are "how many separate sources raised it", which shows a pattern, not a percentage of all users.

## What was read

Read in full (comment text):

- Hacker News, OpenScreen launch thread: <https://news.ycombinator.com/item?id=47595695> (about 50 top level comments, mostly about Screen Studio's price and its Linux and Windows gap)
- Hacker News, Cap launch thread: <https://news.ycombinator.com/item?id=40338275> (Loom as the reference point)
- Hacker News, browser based recorder launch: <https://news.ycombinator.com/item?id=43816419> (recording length, browser support, sharing)
- Screen Studio's own public feedback board: <https://hub.screen.studio/> (about 20 recent posts, including many after its 4.0 update)

Read as titles, comment counts and bodies through the GitHub search API, not every comment:

- Cap issues: [#382](https://github.com/CapSoftware/Cap/issues/382) export audio not in sync, [#593](https://github.com/CapSoftware/Cap/issues/593) audio not in sync, [#1535](https://github.com/CapSoftware/Cap/issues/1535) crash while recording, [#1589](https://github.com/CapSoftware/Cap/issues/1589) excessive RAM, [#342](https://github.com/CapSoftware/Cap/issues/342) black screen after recording, [#1920](https://github.com/CapSoftware/Cap/issues/1920) app does not start on an M1 Air, [#109](https://github.com/CapSoftware/Cap/issues/109) CPU and memory use, [#352](https://github.com/CapSoftware/Cap/issues/352) zoom animations like Screen Studio, [#2928](https://github.com/CapSoftware/Cap/issues/2928) configurable zoom duration and click merge gap, [#1522](https://github.com/CapSoftware/Cap/issues/1522) captions added to cut parts, [#2960](https://github.com/CapSoftware/Cap/issues/2960) filler word removal from word timed captions, [#2328](https://github.com/CapSoftware/Cap/issues/2328) waveforms, bulk actions and gap segments, [#504](https://github.com/CapSoftware/Cap/issues/504) webcam only mode and full size webcam layer, [#1588](https://github.com/CapSoftware/Cap/issues/1588) microphone stays on after recording (Windows 11)

Seen only as search result snippets. Reddit blocked page reads from this machine, so these were **not** read and are listed only as leads for the next round:

- <https://www.reddit.com/r/macapps/comments/1r0a7ve/screen_studio_audio/>
- <https://www.reddit.com/r/Entrepreneur/comments/1q88pep/what_are_good_alternatives_to_loom_for_creating/>
- Screen Studio's developer on a sync bug: <https://pietrasiak.com/debugging-%22multi-issue-bugs%22>
- A competitor's blog on Screen Studio drift (written to sell its own product, so treat it as biased): <https://www.screenkite.com/blog/fix-screen-studio-audio-drift-sync-issues>

Product Hunt, YouTube comments and the Arcade, Supademo and Tella communities were not covered. The sample leans toward developers, because that is who posts on Hacker News and GitHub. It also leans toward Mac users.

## The needs, ranked by how many sources raised them

| # | Need | Sources | Already in Booya before 0.5.0? | What 0.5.0 does |
|---|------|---------|-----------------------------------|-----------------|
| 1 | **Sound and picture stay together** (drift, export out of sync, hover shown before the cursor gets there) | Cap #382, #593; Screen Studio board ("Hover states appear before cursor reaches them"); Screen Studio developer write up; Screen Studio board export failures | Partly. A Mac fix in 0.4.2. | Measured sound against picture at every stage, on every OS, in CI. Mic on the side for Linux and Windows too. Editor preview follows the sound. See `docs/SYNC.md`. |
| 2 | **Do not lose work, do not crash** | Cap #1535, #1589, #342, #1920, #109; Screen Studio board ("Export is failed", background glitch); HN 47647872 (went back to Screen Studio after "many bugs and UX oddities" in two alternatives) | Partly (export has retries) | Autosave of every project every 15 s, recovery offer after a crash, projects saved as a folder with the media inside. See `docs/PROJECTS.md`. |
| 3 | **No subscription, and Windows and Linux** | HN 47595695: at least six comments (price of $29 a month, "subscription for a desktop app", Linux and Windows missing) | Yes. MIT licence, free, macOS, Windows and Linux builds. | Nothing to build. Auto update (signed, from GitHub) keeps it that way without an account. |
| 4 | **Fix a take without redoing everything** ("start over", re-record a part, add a clip in the middle, detect takes) | HN 40338541 and 40339872; Screen Studio board (import audio, separate audio tracks); Cap #2328 (blank and gap segments) | No | **Record here** and **Re-record clip**: record a new take and splice it in at the playhead or over a range, with the captions, zooms and callouts after it moved to match. **Import a video clip at the playhead.** |
| 5 | **Control over auto zoom** (strength, how long it holds, how often it fires, smoothness) | Screen Studio board ("Advanced auto-zoom adjustments", "disable background blur during zooms"); Cap #2928, #352, #1782; HN 47646586 (wants a slider), HN 47646924 (zoom feels dizzying) | Partly: level, speed and steadiness sliders | **Zoom hold** and **Group clicks within** sliders (Style tab > Zoom). |
| 6 | **Text you can fix**: captions, notes, titles | Screen Studio board ("manually add subtitles"); Cap #1522, #2960; HN 40338722 (post capture editing) | Captions per word, titles, watermark. Callout text could only be deleted and re added. | Callout text is now editable in place. `docs/EDITING.md` lists every text element and where to change it. |
| 7 | **Shortcuts and key overlay that match the keyboard** (Fn, Ctrl, Alt, Option, media keys) | The owner's own request. Raised in no public source read here, so no claim is made about demand. | Mac only | Shipped in 0.5.0 (commit add325d). |
| 8 | Sharing with links and view analytics | HN 40338722, 40338429, 40338529; Screen Studio board ("Auto-Email") | Private S3 link upload | Not built. Needs a hosted service, which conflicts with "nothing leaves your computer". |
| 9 | Export data for other editors (cursor as its own layer, click and typing events for Resolve or Premiere) | HN 47646500; Screen Studio board ("Preserve Screen Studio's cursor in advanced editing workflows") | Booya already writes the event log (`.jsonl`) beside every recording | Not built. Candidate for the next round. |
| 10 | Separate audio tracks, import a voice over | Screen Studio board (two posts) | No | Not built. |
| 11 | 1.25x and similar playback speeds, a start countdown | HN 40338722, 40338535 | Countdown yes, 1.25x no | **1.25x** clip speed added. |
| 12 | Long recordings and file size | HN 43816883 | Local files, no limit | Nothing to build. |

## How the top needs were chosen to build

Chosen because (a) several independent sources raise them, (b) Booya did not already cover them, and (c) they can be built and tested without a hosted service:

1. Sync (need 1): built, measured, and covered by CI on three OSes.
2. Autosave and projects (need 2): built.
3. Insert, re-record and import (need 4): built.
4. Zoom hold and click grouping (need 5): built.
5. Editable callout text and a text guide (need 6): built.

Needs 3, 11 and 12 were already met or are one line changes. Needs 8, 9 and 10 are deferred, with reasons above.

## What this sample cannot tell you

- It does not say how common any need is among all users.
- Hacker News and GitHub reward people who like to write about tools. People who record a demo once a quarter are underrepresented.
- Need 7 is the only one in the list that comes from the owner alone.

## Suggested next step for real research

Reddit threads and the Product Hunt pages above were not read. A better source would be five short calls with people who record product demos every week, asking them to show their last recording and what they fixed afterwards. That is a paid or favour job, and nothing was arranged here.
