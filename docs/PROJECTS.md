# Projects

## What a project is

Save a project and Booya writes a **folder** that ends in `.booya`:

```
My demo.booya/
  project.json        everything you set: clips, zooms, captions, callouts, style
  transcript.json     the transcript, when there is one
  media/              the recording, its microphone and webcam parts
  events/             the click, key and cursor log
  assets/             backgrounds and other pictures you used
```

Paths inside `project.json` are relative to the folder, so you can move or zip the folder, or put it on another computer. Files are linked, not copied, when the disk allows it, and a file used twice is stored once.

Old projects (a loose `.json`) still open. **Save** writes a bundle the next time you save.

## Where to find it

- **Project** button in the top bar: Save as, Duplicate, Open a project folder, Import a video clip at the playhead. (On a Mac the same items are in the File menu.)
- The start screen lists recent projects. When a project's files are missing, a dialog lists them with a **Locate...** button for each, so you can point Booya at the new place.

## Autosave and recovery

- Every 15 seconds, when you switch away from the window, and when you quit, but only if something changed.
- A new recording autosaves into its own `recording-...` folder in your recordings folder, so a crash after recording still leaves an editable project.
- A project you saved yourself is never overwritten by autosave. Autosave writes `project.autosave.json` beside it. On opening, if that file is newer and different, Booya asks whether to recover it. Saving removes it.

## Add a recording in the middle, or redo a part

- **Record here** (editor toolbar under the preview, next to Zoom): records a new take and inserts it at the playhead. Everything after it moves later. Captions, zooms, callouts, blur boxes and shapes after the point move with it, and one that spans the point stretches.
- **Re-record clip**: records over the selected clip's range. What was inside is replaced, and what comes after moves to match.
- **Import a video clip at the playhead**: same insert, from a file.
- The old recording is kept (the new one is `name-take1.mp4`), so **Undo** goes back.

Limits:
- Only speed is a per clip setting; there is nothing else to carry over.
- The take's own webcam is not used. If the project had a webcam, it is hidden after a splice, because one offset cannot cover a gap. Turn it back on under Style > Webcam if the result suits you.
- Sound is joined at 48 kHz stereo. A take without sound gets silence.
- Tested on files with the sync probe (offset and drift stay inside the budget after an insert and after a replace). Not tested by hand in a window.
