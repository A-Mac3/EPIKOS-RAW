//! Print a file's lens metadata and embedded lens corrections.
fn main() {
    for path in std::env::args().skip(1) {
        match epikos_decode::decode_file(&path) {
            Ok(d) => {
                let m = &d.metadata;
                println!(
                    "{path}\n  camera {} {} | lens {:?} {:?} | focal {:?} f/{:?} | {}×{} | {:?}",
                    m.make, m.model, m.lens_make, m.lens_model, m.focal_length, m.f_number,
                    d.mosaic.width, d.mosaic.height, d.profile.orientation
                );
                println!("  camera profile: {:?}", d.lens_profile);
            }
            Err(e) => println!("{path}: {e}"),
        }
    }
}
