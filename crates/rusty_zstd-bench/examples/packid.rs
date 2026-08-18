//! Byte-identity gate for the PACKED tag representation, on FULL corpora.
//! The 24-bit position residue only wraps past 16 MiB, so an 8 MiB slice would
//! never exercise it -- the 32 MiB corpora are the point of this gate.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for lvl in [1i32,2] {
        println!("\n=== L{lvl} (FULL files) ===");
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let z=rusty_zstd::compress(&full,lvl).unwrap();
            let ok=rusty_zstd::decompress(&z).unwrap()==full;
            println!("{id:<14}{:>10} MiB  {:>11} bytes  round-trip {}",
                full.len()>>20, z.len(), if ok {"OK"} else {"FAIL"});
            assert!(ok, "{id} L{lvl} round-trip FAILED");
        }
    }
}
