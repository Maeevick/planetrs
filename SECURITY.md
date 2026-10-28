# Security

If you found a hole in [planetrs](https://github.com/maeevick/planetrs), maeevick would very much like
to hear about it before anyone else does.

## Reporting a vulnerability

Report it privately through GitHub: on the repository's **Security** tab, choose **Report a
vulnerability**. Please do not open a public issue for it.

Say what you found, which version and command it affects, and how to reproduce it. You will get an
answer within a week, and a fix or a plan for one as soon as the problem is understood. With your
agreement, the advisory credits you.

## What is in scope

- The `planetrs-geocoder` and `planetrs-mapclipper` commands and the published crates.
- Reading untrusted input: site files, planet files, harvests, themes and dataset files.
- Downloads: either command's `download` fetches over HTTPS and refuses a file whose md5 does not
  match the one published beside it.

Only the latest release is supported.
