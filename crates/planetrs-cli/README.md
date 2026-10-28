# planetrs-cli

What the [planetrs](https://github.com/maeevick/planetrs) command lines share, so that both tools
keep their data in the same place, never write over an output, draw progress the same way and end the
same way:

- the data directory: `--data-dir`, then `PLANETRS_DATA_DIR`, then `$XDG_DATA_HOME/planetrs`, then
  `~/.local/share/planetrs`, holding `planets/` and `dataset/`;
- the planet's download: resumed after a break, checked against the md5 published beside it, and
  named after the Monday its header states;
- outputs written whole under a free name, never over an existing file, with a dated name when the
  one asked for is taken;
- one progress line on stderr, drawn only when someone is watching;
- exit codes: 0 on success or a closed pipe, 1 for a failure, 2 for a command asked for wrongly, and
  the OpenStreetMap attribution every run that reads its data ends with.

The `testing` feature brings a small web server serving files by range, for the tests of downloads.

## Licence

The code is MIT ([LICENSE](https://github.com/maeevick/planetrs/blob/main/LICENSE)). The data both
tools read is © OpenStreetMap contributors, under the Open Data Commons Open Database License 1.0.
