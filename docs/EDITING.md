# How to edit every piece of text

Everything below exists in the app. Paths are tab, then section. The right hand panel has four tabs: Auto edit, Style, Edit, Export.

## Subtitles and captions

Needs a transcript first: **Auto edit > Tools > Transcribe** (or tick Captions in Auto edit and press Run auto edit).

**Edit tab > Transcript**
- Click a word to change it. Enter saves, Tab goes to the next word, Shift+Tab to the previous one, Escape cancels. The word keeps its timing.
- Replace several words at once: type the word in **Find**, the new one in **Replace with**, press **Replace all**.
- Each line has buttons: **Split** (starts a new line at the word you last clicked), **Merge next**, **Start -0.1s / +0.1s**, **End -0.1s / +0.1s**, **Move -0.1s / +0.1s**, **Cut line** (removes that part of the video).
- The time at the left of a line jumps the playhead there.

Show them in the video: **Style tab > Keys and captions > Burn in captions**, size with **Caption size**.
Take them out as a file: **Auto edit > Tools > Export subtitles** (SRT or VTT). Your edits are what gets written.

## Callouts (short notes on the video)

**Edit tab > Callouts**
- Type the text in the box, choose Bottom, Middle or Top and 2, 3 or 5 s, press **Add**. It is placed at the playhead.
- In the list below, the text of every callout is a box. Change it and press Enter or click away. There is a button to jump to its time and one to remove it.

## Title cards (intro and outro)

**Style tab > Intro and outro**: Intro title, Intro subtitle, Outro title, Outro subtitle, and Card length. Leave the title and subtitle empty for no card. These show in the exported video, not in the editor preview.

## Watermark

**Style tab > Watermark**: Text, Corner, Opacity. Empty text means no watermark.

## Keystroke overlay

**Style tab > Keys and captions**: Keystroke overlay on or off; **Hide typing, keep shortcuts and special keys**; on a Mac, **Mac symbols** instead of words. The overlay text comes from what was pressed during the recording, so it cannot be retyped. You can hide it or turn it off.

## Shapes

**Edit tab > Shapes** (arrow, box, ellipse, highlight) have no text. For a label, add a callout.

## Names of things that are not video text

- Shortcut keys: **Settings > Shortcuts**. Click the keys, press the new combination, Esc cancels.
- Project name: choose it when you save (**Project > Save as**).
- Settings is the gear button at the top right.

## Not editable today

- The wording of the key overlay labels.
- Fonts and colours of callouts and cards.
- Per shape labels.

## Auto zoom: when it zooms

Auto zoom is selective. It scores what happened and only moves the camera when the viewer has something small to look at. Every zoom it makes is a normal segment on the timeline: drag it, resize it, change its scale or delete it.

**What scores**

| Moment | Score |
| --- | --- |
| A marker you dropped | 1.0 |
| Typing starts (3 or more keys with under 1.2 s between them) | 0.8 |
| A click where the cursor had settled first (an aimed click) | 0.75 |
| The cursor arrives somewhere and rests about 1 s | 0.6 |
| A click while the cursor was still moving | 0.4 |

Events that follow each other (within 3 s and within a quarter of the screen, or within 1.8 s wherever they are) form one group. A group zooms when its best score plus 0.15 per extra event reaches the threshold of the chosen intensity.

**What never zooms**

- Clicks in the menu bar, the tab strip, the Dock or the taskbar, and at the screen edges.
- Right and middle clicks, and drags.
- Clicks within 1.2 s after a tab or window switch (Cmd or Ctrl plus Tab, T, W, N, a digit, Page Up, Page Down, the arrows or the backtick key).
- A click straight after the cursor jumped across the screen.
- The second click of a quick double click (under 0.35 s).
- Shortcuts. They do not start a zoom on their own.

**Staying in, panning and zooming out**

- If the next event is within a quarter of the screen (about 18 percent for Subtle, 30 percent for Lively), the camera stays zoomed and pans to it.
- If the next event is far away but comes soon (under 1.8 s), the camera goes straight there. It does not zoom out and in again.
- After 2 to 3 s without activity (the Zoom hold slider) it eases out.
- Zooms start about 0.6 s before the event, are at least 4.4 s long and are never closer than 4.5 s start to start (Normal). The better one stays when two collide.

**Intensity**

| | Subtle | Normal | Lively |
| --- | --- | --- | --- |
| Needs a score of | 0.8 | 0.55 | 0.35 |
| Level | 1.5x | 1.6x | 1.8x |
| One zoom per at least | 5.5 s | 4.5 s | 3.5 s |
| Dwell time | 1.5 s | 1 s | 0.8 s |

Subtle zooms on typing, markers and clusters of clicks. Normal also zooms on a single aimed click and on a cursor that rests. Lively also zooms on hurried clicks. The depth is never more than 2x and always keeps a margin round what was clicked. The camera uses a spring curve: it leaves at once, eases to a stop and never overshoots.

**Limits.** "Small target" is judged by the cursor settling before a click, because the event log has no information about what is under the cursor. Scroll events are not logged, so scrolling cannot be skipped by rule, but it moves no clicks and so makes no zoom.
