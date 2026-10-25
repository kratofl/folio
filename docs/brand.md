# Folio brand: Safelight

Named after the red-orange light in a darkroom. The app's tokens live in `crates/folio/src/theme.rs`. This page is the full set and the rules behind it.

## Mark and icon

The mark is an F built from photo tiles in justified rows, like the grid in the app. Two tiles at half strength stand for photos not picked yet.

- `assets/brand/folio-mark.svg`: the full mark, for 32 px and up.
- `assets/brand/folio-mark-small.svg`: drops the faded tiles and widens the gaps, for 32 px and below.
- `assets/brand/folio-icon.svg`: the app icon, a Safelight gradient plate with a white mark. Windows uses the same artwork without the rounded mask.

The wordmark is lowercase "folio" in a heavy, tightly spaced sans, with the mark to its left.

## Color

Brand color stays out of the photos. The grid and loupe are pure neutral black, and Safelight only marks what you act on.

| Token | Hex | Use |
| --- | --- | --- |
| accent | `#FF5A36` | Selection, stars, primary buttons |
| accent.hover | `#FF7452` | |
| accent.pressed | `#E23D1C` | |
| accent.soft | `#FFB38F` | Text on dark accent areas |
| on.accent | `#16110F` | Text on accent |
| canvas | `#000000` | Photo grid, loupe |
| tile.empty | `#121212` | Tile without a thumbnail yet |
| surface | `#16110F` | Rail, welcome screen |
| raised | `#201A17` | Dock, sheets, menus |
| line | `#2E2622` | Borders, separators |
| hover | `#3D332E` | |
| text | `#F6EDE6` | |
| text.muted | `#B3A79F` | |
| text.dim | `#7A6E68` | |
| text.off | `#4D4440` | Empty stars |

The warm darkroom tint (surface, raised, line) only goes on chrome that floats above the grid, never behind photos.

Color labels sit away from Safelight so a label never reads as the selection. Red moved toward crimson for that reason.

| Label | Hex |
| --- | --- |
| red | `#E0366F` |
| yellow | `#E9C440` |
| green | `#43B76A` |
| blue | `#3F8EF2` |
| purple | `#A274F2` |

## Type

Inter for the interface, Inter Display for big headings. Both are bundled from `crates/folio/assets/fonts/` under the SIL Open Font License (`OFL.txt` there).

| Style | Font | Size |
| --- | --- | --- |
| display | Inter Display ExtraBold | 26 |
| title | Inter Bold | 17 |
| body | Inter Regular | 13 |
| small | Inter Medium | 12 |
| caption | Inter Medium | 11 |
