//! HYPOTHESIS: packing breaks not because of the 24-bit wrap, but because Gate 1
//! routes versions-16m from find_fast (packed writer) to find_lazy (PLAIN reader
//! via get_h) MID-FRAME, on the SAME MatchTables.
//!
//! PREDICTION: disable Gate 1 and the packed build becomes byte-identical on
//! versions-16m. If the wrap were the cause, disabling Gate 1 would change
//! nothing.
fn main(){
    for id in ["versions-16m","text-32m","mozilla","webster"] {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        for (label, gate1) in [("Gate 1 ON (default)", true), ("Gate 1 OFF", false)] {
            rusty_zstd::set_fast_lazy_arm(gate1);
            let z=rusty_zstd::compress(&full,1).unwrap();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(), full, "{id} round-trip");
            println!("{id:<14}{label:<22}{:>11} bytes", z.len());
        }
    }
    rusty_zstd::set_fast_lazy_arm(true);
}
