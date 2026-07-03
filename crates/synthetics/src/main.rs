//! Thin daemon entrypoint. Real wiring lands in T10 (`daemon` module);
//! for now this only proves the binary links against the library.

fn main() {
    println!("{} (scaffold)", synthetics::NAME);
}
