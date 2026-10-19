//! Serves a dataset's directory over HTTP range requests on this machine, as its host would once it
//! is published, so that a remote dataset can be tried before it is:
//!
//! ```sh
//! cargo run -p planetrs-dataset --features testing --example serve -- ~/.local/share/planetrs/dataset
//! planetrs-geocoder --dataset http://127.0.0.1:PORT/ "label;address;postal code;city;country"
//! ```

use planetrs_dataset::testing::RangeServer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args().nth(1).ok_or("name the dataset's directory")?;
    let files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .map(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            std::fs::read(&path).map(|bytes| (name, bytes))
        })
        .collect::<Result<_, _>>()?;
    let served: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let server = RangeServer::serving(&served)?;
    println!("{}", server.url());
    loop {
        std::thread::park();
    }
}
