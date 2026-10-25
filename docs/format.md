# Library format

A Folio library is a plain folder of photos. Folio keeps as little of its own state as it can, and puts each piece where other apps and file sync handle it best.

## Folders are the organisation

- A photo in `Category/Outing/` is sorted. `Category` is the top folder, for example `Events/Christmas market/`.
- A photo in the library root, in a date folder (`2024`, `2024-10-20`, `20241020`) or in a camera folder (`DCIM`, `100NIKON`) is unsorted, at any depth. `2024/2024-10-20/` is unsorted, `2024/Stuttgart/` is sorted under the category `2024`.
- Folder names typed in Folio have the characters Windows forbids (`<>:"/\|?*`) replaced with `-`, lose trailing dots and spaces, and are stored as NFC.
- Folders match without case. Typing `events` files photos into an existing `Events`.
- Videos (`.mov`, `.mp4`, `.m4v`) in unsorted folders move with the outing they were shot during: the one closest in time, within the two-hour gap that separates outings. Their time comes from the QuickTime movie header, which is UTC, converted to local time to match the photos' EXIF. Videos without a time stay where they are.

## Marks live in XMP sidecars

The sidecar next to a photo (`_DSC0001.xmp` for `_DSC0001.NEF`) is the source of truth for marks, so Lightroom, Bridge, darktable and others see them too.

| Mark | Property | Values |
| --- | --- | --- |
| Stars | `xmp:Rating` | `1` to `5`, `0` for none |
| Color label | `xmp:Label` | `Red`, `Yellow`, `Green`, `Blue`, `Purple` |
| Pick or reject | `folio:Flag` | `pick`, `reject`, absent for none |

`folio` is the namespace `https://github.com/kratofl/folio/xmp/1.0/`.

- Folio patches only these three properties in the sidecar's text and leaves every other byte as it was, including Camera Raw settings. The new text goes to a hidden temporary file that then replaces the sidecar in one rename.
- Photos without a sidecar get a minimal one on their first mark.
- Reading also understands `xmp:Rating="-1"` (Bridge's reject) and Lightroom's German label names.
- RAW files are never written.

### Sync conflict copies of sidecars

When two machines change the same sidecar offline, sync services keep both: `_DSC0001 2.xmp` (iCloud), `_DSC0001 (conflicted copy ...).xmp` (Dropbox, Nextcloud), `_DSC0001 (1).xmp` (Google Drive), `_DSC0001.sync-conflict-....xmp` (Syncthing). Folio merges them when the library opens.

- A sidecar whose name matches a photo is never treated as a copy, so `Photo 2.xmp` next to `Photo 2.NEF` stays that photo's sidecar.
- Marks merge one by one. A mark set in only one file is kept. A mark set to different values in both takes the newer file's value. A cleared mark can come back this way, which beats losing a rating someone set.
- Copies that differ in anything besides the three marks (a Lightroom edit, for example) can't merge without losing work. Both files stay, and Folio says so when the library opens.
- If only a copy exists, it's renamed to the sidecar's name.
- The merged marks are written before the copy is removed, so a crash in between just merges again next time.

## `.folio/` holds what has no other home

```text
.folio/
  moves/
    1729871234123.tsv
  outings/
    be67af8eb565f861.txt
  settings.txt
  views/
    Best of October.txt
```

### Move journals

Every batch of moves gets one journal, named by the time it started in milliseconds so the names sort by age. One file per batch means two machines never edit the same journal.

```text
folio moves 1
2024/2024-10-20/_DSC0001.NEF<TAB>Events/Fair/_DSC0001.NEF
2024/2024-10-20/_DSC0001.xmp<TAB>Events/Fair/_DSC0001.xmp
done
```

- The first line is the format version. Each move is `from` and `to`, relative to the library root, NFC, `/` separated, split by a tab.
- The journal is written and flushed before the first file moves. After that, Folio only appends state lines: `done`, `undoing`, `undone`, `redoing`. The last one wins.
- A journal with no state line, or ending in `redoing` or `undoing`, was interrupted. On open, Folio finishes it in the direction it was going. Every move checks the disk first and skips files that already moved, so finishing twice is harmless.
- A move never overwrites an existing file. Name clashes in the target folder are solved while planning: `_DSC0001-2.NEF`, with the sidecar renamed to match.
- Undo takes the newest `done` journal. Redo takes the newest `undone` one, as long as no batch ran after it.
- Folders left empty by a move or an undo are removed. A folder holding anything else, even `.DS_Store`, stays.
- Paths in a journal resolve without case and Unicode form, so a journal written on one machine still finds its files on the other.
- A conflict copy of a journal (`1729871234123 2.tsv`) lists the same moves, since moves are written once. Only the state can differ, and the disk settles it: if every file is where the batch put it the state is `done`, if every file is back it's `undone`. Folio appends that and removes the copy. When the files are half moved, or the copy lists other moves, both are kept and reported.

### Saved views

A saved view is a named filter, one file per view, so editing one view never touches another's file.

```text
folio view 1
name<TAB>Best of October
rating<TAB>>=3
flag<TAB>pick
lens<TAB>NIKKOR Z DX 16-50mm f/3.5-6.3 VR
```

- Conditions are `rating` (`>=3`, `=3`, `none`), `flag` (`pick`, `reject`, `none`), `label` (a label name or `none`), `camera`, `lens`, `folder` (relative, includes subfolders) and `date` (`2024` or `2024-10`).
- Conditions of the same field widen each other, different fields narrow. Two lenses show photos from either lens, a lens and a rating show photos with both.
- Unknown lines are skipped, so a view from a newer Folio still opens.
- Two files with the same `name` are a sync conflict copy. The newer file wins and the older one is removed. A view is one value, its list of conditions, so there's nothing finer to merge. A file that isn't a view is reported when the library opens and left alone.

### Outing drafts

An outing the user named, gave a category, accepted, split or merged is saved as a draft until its photos move. Outings nobody touched aren't saved, since grouping makes them again.

```text
folio outing 1
name<TAB>Christmas market
category<TAB>Events
accepted
photo<TAB>2024/2024-12-07/_DSC0001.NEF
photo<TAB>2024/2024-12-07/_DSC0002.NEF
```

- One file per outing, named after a hash of its first photo's path (without case, NFC). Two machines editing different outings never touch the same file.
- On open, outings are grouped as usual, then each draft takes its photos back out of them. Photos that moved or are gone are skipped, and a draft left with none is removed.
- Only drafts that changed are written. A draft is removed once its photos move, and comes back on undo with the folder's name.
- Two files for the same first photo are a sync conflict copy. The newer one wins and the older is removed. A draft is one decision, the outing as the user last left it, so there's nothing finer to merge.

### Settings

Settings that belong to the library, not the machine, so every machine groups the same photos the same way.

```text
folio settings 1
outing-gap<TAB>120
```

- `outing-gap` is how many minutes without photos start a new outing. Without it, two hours.
- Unknown lines are kept when saving and skipped when reading, so a file from a newer Folio still works.
- Saving re-reads the file and changes only its own line, never writing a value from stale memory.
- A conflict copy (`settings 2.txt`) merges per value: each setting comes from the newest file that has it. The result is saved and the copy removed. A copy that can't be read is reported and left alone.

## Not stored

- Thumbnails live in the OS cache folder (`~/Library/Caches/Folio` on macOS, `%LOCALAPPDATA%\Folio\Cache` on Windows), keyed by file name, size and modification time. Deleting them only costs time.
- The list of recently opened libraries lives in the app-data folder (`~/Library/Application Support/Folio`, `%APPDATA%\Folio`), since it's about the machine, not the library.
