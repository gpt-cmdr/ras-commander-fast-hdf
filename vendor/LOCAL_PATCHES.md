# Local reader patches

`vendor/` holds a modified copy of the `hdf5-reader` and `hdf5-core` crates from
[roteiro-gis/netcdf-rust](https://github.com/roteiro-gis/netcdf-rust), based on upstream
commit `24ff6fcfcc79a35c30aa5d8cb3028ab4d9d00564` (crate version 0.9.1). The upstream
licences (`LICENSE-MIT`, `LICENSE-APACHE`) and upstream `README.md` are retained unchanged.
None of these changes has been submitted upstream yet.

The full change notice, listing every modified file and the reason for each change, is
[`hdf5-reader/PROVENANCE.md`](hdf5-reader/PROVENANCE.md). Per-file SHA-256 hashes of the
vendored tree are recorded in [`PROVENANCE.json`](PROVENANCE.json) and checked by
`scripts/prepare-reader.ps1`.

## Patch files

The first four changes are also kept as unified diffs in `patches/`. They apply in order,
from the `vendor/` directory, with `git apply`, starting from the upstream base.

| Patch | Change |
|---|---|
| `01-dependency-shared-message.patch` | `messages/shared.rs`: resolve shared object-header messages. |
| `02-dependency-fixed-array-width.patch` | `fixed_array.rs`: correct the width of the chunk-size field in filtered fixed-array entries. |
| `03-metadata-raw-encoding.patch` | Keep exact datatype envelopes (`DatatypeMessage.raw_encoding`, `AttributeMessage.raw_datatype`); add non-eager `Group::raw_attributes` and `Dataset::raw_datatype`; add `Error::UnsupportedEncoding`; bound attribute-message parsing. |
| `04-dataset-raw-attributes.patch` | Add `Dataset::raw_attributes()`, the dataset counterpart of `Group::raw_attributes`. |

Notes on patches 03 and 04:

- `DatatypeMessage.raw_encoding` and `AttributeMessage.raw_datatype` are new required
  fields, which is a breaking change for external struct-literal constructors.
  `Group::raw_attributes`, `Dataset::raw_datatype` and `Dataset::raw_attributes` are
  additive APIs.
- The engine proves raw numeric and fixed-string encodings before decoding. All
  variable-length metadata remains unsupported before dereference. The generic high-level
  variable-length APIs keep their previous behaviour and are not certified safe by these
  patches. Arrays, references and nested metadata remain unsupported.
- `Dataset::raw_attributes` returns a borrowed `&[AttributeMessage]` (infallible) while
  `Group::raw_attributes` returns `Result<Vec<AttributeMessage>>`, because dataset
  attribute messages are already parsed when the dataset is opened whereas group headers
  are parsed lazily.
- Neither patch contains HEC-RAS-specific names or schema.

## Later changes

Changes made after patch 04 exist only in the vendored source (there are no separate
`.patch` files for them). They are described in `hdf5-reader/PROVENANCE.md`. In summary:

- an old-style (symbol-table) group name cache in `FileContext`, keyed by group address,
  storing only names and addresses; the local heap segment is read once per first
  enumeration with checked UTF-8, offset and terminator handling; and
  `Group::member_names`, which lists object kinds without materialising child datasets;
- sparse row/column gathers from chunks, a cache bypass for large reads and one index
  traversal for parallel cold gathers;
- refusal of unqualified non-inline extensible-array layouts;
- bounded filter pipelines and a configurable per-chunk allocation limit;
- tolerance of stale chunks wholly outside a shrunk dataset's extent;
- soft links in old-style groups, with one shared depth, cycle and expansion budget;
- checked arithmetic for file-derived sizes, a panic-safe chunk cache, and attribute
  element counts checked against the stored payload before allocation.
