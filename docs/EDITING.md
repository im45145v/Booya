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

## Camera layouts

When a recording has a camera, the **Style** tab has a **Layout** choice for the whole recording and the timeline gets a **LAYOUT** lane:

- **Screen only** hides the camera.
- **Camera bubble over the screen** is the usual round or square picture in a corner.
- **Side by side** puts the camera in a column next to the screen (above or below it in a tall video), so nothing is covered.
- **Camera full screen** shows only the camera.

To change the layout for part of the recording, double click an empty place on the LAYOUT lane (or press **Add at playhead**). A block appears; drag it to move it, drag its edges to resize it, choose its layout under **This stretch**, and press Delete to remove it. Blocks cannot overlap. Where there is no block the whole recording layout applies. **Blend** sets how long the picture takes to glide from one layout to the next (0 cuts straight over).

In a 9:16 or 1:1 video the whole screen would be tiny, so the screen shows a window that follows the cursor and blends towards the zoom. Turn off **Tall and square videos follow the cursor** to show the whole screen with bars instead.

## Names of things that are not video text

- Shortcut keys: **Settings > Shortcuts**. Click the keys, press the new combination, Esc cancels.
- Project name: choose it when you save (**Project > Save as**).
- Settings is the gear button at the top right.

## Not editable today

- The wording of the key overlay labels.
- Fonts and colours of callouts and cards.
- Per shape labels.

## Auto zoom: when it zooms

The camera stays on the whole screen unless something small is worth a closer look. A recording opens full screen, and most of a typical minute stays that way: expect 3 to 6 zooms a minute, usually fewer. Every zoom is a normal segment on the timeline: drag it, resize it, change its scale or delete it.

**What makes a zoom**

- **A cluster of clicks.** At least 3 left clicks (Normal) that land inside a small area, about a fifth of the screen, within 4.5 s of each other. The count and the window are settings (Auto zoom > Clicks to zoom, Within). Subtle needs 4 clicks, Lively needs 2.
- **Typing.** A burst of at least 5 keys that goes on for more than 1.5 s, in one place.
- **A marker** you dropped during the recording.

That is all. A single click does not zoom, and neither does a cursor that rests, a window that slides in, the screen changing or a cursor that moves a long way. Those used to count and made the camera jump for no reason.

**What never counts**

- Clicks in the menu bar, the tab strip, the Dock or the taskbar, and at the screen edges.
- Right and middle clicks, and drags.
- Clicks within 1.2 s after a tab or window switch, and the second click of a quick double click.
- Clicks while the page is scrolling.
- Anything in the first second.

**Staying in, panning and zooming out**

- Clicks that keep coming in the same area stay in the running zoom, and the camera pans to them inside a dead zone (the Steadiness setting), so small cursor motion does not move the view.
- The camera eases in and out on a spring curve (about 0.9 s each way), holds 2.5 s after the last click or key, and never overshoots.
- The whole screen stays up at least 1.5 s between two zooms.
- **Scrolling.** Scroll events are logged on every system. While you scroll, the camera eases out to the full screen and stays there until the scrolling has stopped for 0.8 s.

**Intensity**

| | Subtle | Normal | Lively |
| --- | --- | --- | --- |
| Clicks in one area | 4 | 3 | 2 |
| Level | 1.5x | 1.6x | 1.8x |
| Hold after the last event | 2 s | 2.5 s | 3 s |

The depth is never more than 2x and always keeps a margin round what was clicked.

**Limits.** "Small area" is judged from where the clicks landed, because the event log has no information about what is under the cursor. Typing is detected from the key events, so with key capture off there are no typing zooms.

## Device frames, motion blur, music and muting a clip

- **Device frame** (Style > Frame): a simple vector laptop, browser window or phone drawn around the recording. They are plain shapes, not any maker's artwork. Choose None to keep the plain or glass look.
- **Motion blur** (Style > Frame): blurs the picture along the direction the camera is moving during zooms and pans. Off by default; the picture is only smeared while the camera is moving.
- **Music** (Style > Music): pick an audio file. Set the volume, how far it ducks under your voice (and by how many dB), the fade in and out, where it starts and whether it loops. The editor plays it natively, the export mixes the same way.
- **Mute a clip** (Edit tab): select a clip and tick Mute clip. The recorded sound (voice and system audio) in that clip goes silent and the picture stays. The music keeps playing.
- **Camera position**: the camera bubble can sit in any of the four corners or the middle of any edge.
