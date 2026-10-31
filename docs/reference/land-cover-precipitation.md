# Final n Values and gridded precipitation

Two raster inputs that used to need HEC-RAS or a Python read loop. The engine reads the HDF
files and computes the values. It has no raster I/O: the caller reads the land-cover TIFF
window and writes the GeoTIFFs, with terrio or rasterio.

## Final n Values

`land_cover::FinalManningsModel` (Python `FinalManningsModel`) is RAS Mapper 7.0.1's "Final n
Values" layer, on its default *Export Raster* grid: the geometry extent, at the land-cover TIFF
cell size. In order, each pixel:

1. starts at no data (-9999);
2. takes the 2D flow area's Manning's n, if inside a perimeter;
3. takes its land-cover class from the nearest TIFF pixel (class 0 is no data), then from the
   classification polygons;
4. takes its calibration region (first region wins);
5. takes the class's base n, or the region's override, if that is a valid n. The calibration
   table's base value replaces the land-cover value when it is valid.

RAS Mapper skips step 5 for the last pixel of each 256 x 256 export tile. `TileCorner::RasMapper`
reproduces that. `TileCorner::Evaluate` applies the rule to every pixel.

Call `export_grid(cell_size)`, then for each `tile_rows` row read `class_window(...)` from the
TIFF and pass it to `render_tile_row(...)`. Memory is one row of tiles, not the raster.

### Refused

These raise `Unsupported` rather than produce a different raster:

- cross sections, river centerlines or pipe networks (RAS Mapper also maps 1D n values);
- virtual (merged) land-cover layers;
- multi-part polygons and open rings;
- class IDs above 255;
- more than one new class from classification polygons (RAS Mapper's ID order is unproven);
- non-ASCII class names (case-insensitive matching is unproven);
- a 2D flow area without a computed mesh;
- the HEC-RAS 5.0.3 calibration region layout.

### Evidence

Against RAS Mapper 7.0.1 *Export Raster* output, public HEC-RAS examples, every pixel:

| Model | Grid | Pixels differing |
|---|---|---|
| BaldEagleCrkMulti2D g01 | 11872 x 8139 | 0 |
| BaldEagleCrkMulti2D g09 | 11881 x 8143 | 0 |
| BaldEagleCrkMulti2D g11 (two 2D areas) | 11872 x 8139 | 0 |
| BaldEagleCrkMulti2D g13 | 11872 x 8139 | 0 |
| Weise_2D g08 (version-1 land cover, bitfield IDs, two regions) | 1651 x 101 | 0 |

g09 took about 2.2 s and 161 MB against about 45 s in RAS Mapper. The Bald Eagle land cover has
one classification polygon (a new class, Main Channel), so that branch is qualified on one polygon
only.

## Gridded precipitation

`precipitation::PrecipitationSteps` (Python `PrecipitationSteps`) yields incremental and
cumulative grids per timestep from `Event Conditions/Meteorology/Precipitation`, processed
`Values` or `Imported Raster Data/Values`. The rules are those of ras2cng's `precipitation.py`:

- cells within `np.isclose` of `NoData` are missing;
- processed values are timestep amounts, and cumulative grids are their running sum (missing
  counts as zero; a cell never valid stays missing);
- `Data Type` starting with `cumul` is differenced instead;
- unit conversion multiplies the float32 result in float64.

It reads one chunk row at a time. Steps arrive in time order whatever the requested order,
because a cumulative grid needs every earlier step. A timestep over `max_bytes` is refused.

### Evidence

BaldEagleCrkMulti2D p06 computed by HEC-RAS 7.0.1 (122 steps, 515 x 522, 244 GeoTIFFs), native
units and mm, written with terrio: 0 of 65,594,520 pixels differ from ras2cng's output, with the
same georeferencing, tags and band descriptions. 2-3 s and 153 MB against 6-7 s and 1,011 MB.
The cumulative-source and `NoData` branches are covered by synthetic fixtures only, because no
available real plan has them.
