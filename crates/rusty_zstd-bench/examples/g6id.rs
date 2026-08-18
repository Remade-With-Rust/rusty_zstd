//! Byte-identity gate for the hoist: same bytes, all 18, at L1.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    rusty_zstd::set_pair_on_arm(true);
    rusty_zstd::set_pair_gain_arm(0.0);
    let mut bad = 0;
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8*1024*1024)];
        let z = rusty_zstd::compress(src,1).unwrap();
        let ok = rusty_zstd::decompress(&z).unwrap()==src;
        if !ok { bad+=1; }
        println!("{id:<14}{:>12} bytes  round-trip {}", z.len(), if ok {"OK"} else {"FAIL"});
    }
    println!("\n{}", if bad==0 {"all round-trips OK"} else {"FAILURES"});
}
