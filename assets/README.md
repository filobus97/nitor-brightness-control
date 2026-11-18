# Assets

- `nitor.manifest`: the Windows application manifest.
- `world-110m.bin` (Nitor Plus only): the land outline of the offline map picker in
  Settings, embedded in the exe with `include_bytes!` (about 20 KB).

## world-110m.bin

Source: [Natural Earth](https://www.naturalearthdata.com/) 1:110m physical vectors,
`ne_110m_land` (public domain; see <https://www.naturalearthdata.com/about/terms-of-use/>).
The GeoJSON was taken from the CC0 mirror
<https://github.com/martynafford/natural-earth-geojson> (`110m/physical/ne_110m_land.json`).

Conversion: every ring of every polygon (outer rings and holes) is rounded to whole
hundredths of a degree, consecutive duplicate points and the closing point are dropped,
and the result is written little-endian as

```
u16 ring_count
per ring: u16 point_count, then point_count pairs (lon, lat) of i16
```

The first pair of a ring is absolute, every other pair is the difference to the previous
one (i16 wrap-around arithmetic, since Antarctica spans -180..180). Rings are drawn with
the even-odd fill rule, which makes the holes work. The reader is
`auto::map::decode_rings`, covered by unit tests.

To regenerate, run a small script that does exactly the above on `ne_110m_land.json`.
