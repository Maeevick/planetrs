//! The `planetrs-geocoder` command line, kept in a library so each piece is tested without a process.
//!
//! The binary parses the command line, hands it to [`run`], and turns the outcome into an exit code
//! with [`exit_code_of`]: answers on stdout, everything else on stderr.
