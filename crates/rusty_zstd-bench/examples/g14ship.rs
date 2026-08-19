//! GATE 14 @ L3 candidate: raise ONLY the second-candidate cut. Full 18, both
//! levels DFast serves (L3 and L4), with round-trip.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    for g2 in [16usize,24,32]{
        for lvl in [3i32,4]{
            let (mut ta,mut tb,mut pa,mut pb)=(0i64,0i64,0u64,0u64);
            let (mut worst,mut wid)=(0f64,"");
            for id in IDS{
                let Some(full)=load(id) else{continue};
                let src=&full[..full.len().min(2<<20)];
                rusty_zstd::set_dfast_good_ml2_arm(0);
                let _=rusty_zstd::take_mm();
                let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
                pa+=rusty_zstd::take_mm().0;
                rusty_zstd::set_dfast_good_ml2_arm(g2);
                let _=rusty_zstd::take_mm();
                let z=rusty_zstd::compress(src,lvl).unwrap();
                pb+=rusty_zstd::take_mm().0;
                assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip {id} L{lvl}");
                let b=z.len() as i64;
                let d=100.0*(b-a) as f64/a as f64;
                if d>worst {worst=d; wid=id;}
                ta+=a; tb+=b;
            }
            println!("cand2={g2:>3} L{lvl}: size {:>+9.4}%   probes {:>+7.2}%   worst {} {:+.4}%",
                100.0*(tb-ta) as f64/ta as f64,
                100.0*(pb as f64-pa as f64)/pa as f64,
                if wid.is_empty(){"none"}else{wid}, worst);
        }
    }
    rusty_zstd::set_dfast_good_ml2_arm(0);
}
