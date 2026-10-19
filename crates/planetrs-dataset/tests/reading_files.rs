//! Every way of reading a dataset's files held to the same contract, on real files.

use std::path::PathBuf;

use planetrs_dataset::testing::{RangeServer, honours_for_reading_files};
use planetrs_dataset::{DirectoryFiles, HttpFiles};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("planetrs-dataset-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

#[test]
fn given_files_in_a_directory_when_read_then_they_honour_the_contract() {
    honours_for_reading_files(|holding| {
        let directory = scratch("directory");
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        for (name, bytes) in holding {
            std::fs::write(directory.join(name), bytes).expect("a scratch file");
        }
        DirectoryFiles::at(directory)
    });
}

#[test]
fn given_files_on_a_web_server_when_read_then_they_honour_the_contract() {
    honours_for_reading_files(|holding| {
        let server = RangeServer::serving(holding).expect("a server on this machine");
        HttpFiles::at(&server.url())
    });
}
