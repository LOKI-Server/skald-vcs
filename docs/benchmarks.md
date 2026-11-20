
# Storage and transfer benchmark

From the Loki VCS workspace, run the release-mode benchmark against a source directory. An optional Git executable runs the comparison in isolated temporary repositories. It is a benchmark dependency only.

~~~powershell
cargo run --release -p loki-vcs-client --example benchmark -- 'PATH_TO_LOKI/crates/loki-server/src' 'C:/Program Files/Git/cmd/git.exe'
~~~

The benchmark stages and commits the source files, then prepends one line to the first sorted file. The binary case creates a 32 MiB deterministic, high-entropy asset and inserts 31 bytes near the beginning. Disk measurements count bytes under each object store after the corresponding revision, excluding working copies and reference metadata. Git runs gc before disk measurements. Transfer figures use Loki VCS's unique uncompressed object payloads and Git's thin-pack payload. They exclude HTTP or protocol framing. Timings cover stage and commit on the local machine.

One Windows x86-64 run produced these values. The result depends on the corpus, edit, machine, and compression.

| Corpus | System | Initial store (bytes) | Store after edit (bytes) | Initial payload (bytes) | Incremental payload (bytes) | Initial commit (s) | Edit commit (s) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| LOKI server source | Loki VCS | 218,110 | 227,798 | 706,591 | 26,661 | 0.587 | 0.075 |
| LOKI server source | Git | 198,671 | 199,056 | 194,785 | 300 | 0.190 | 0.088 |
| 32 MiB binary | Loki VCS | 33,564,616 | 33,766,889 | 33,564,503 | 202,269 | 0.288 | 0.122 |
| 32 MiB binary | Git | 33,567,330 | 33,568,644 | 33,564,884 | 1,181 | 0.865 | 0.797 |

The native binary update reused over 99% of its initial payload and took less local commit time in this run. Git's delta pack sent fewer bytes for both edits. Loki VCS protocol v1 sends missing objects directly, and its chunk reuse operates independently of the receiver's compression choices.
