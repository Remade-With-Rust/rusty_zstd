//! GATE 14 @ L19 dispatch: size + probes across all 18, all bt levels.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    for lvl in [13i32,16,19,22]{
        let (mut a,mut b,mut pa,mut pb)=(0i64,0i64,0u64,0u64);
        let (mut w,mut wid)=(0f64,"");
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(8<<20)];
            rusty_zstd::set_bt_deep_min_arm(f32::MAX);   // dispatch off
            let _=rusty_zstd::take_bt_probe_stats();
            let x=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            pa+=rusty_zstd::take_bt_probe_stats().0;
            rusty_zstd::set_bt_deep_min_arm(2.0);        // shipped
            let _=rusty_zstd::take_bt_probe_stats();
            let z=rusty_zstd::compress(src,lvl).unwrap();
            pb+=rusty_zstd::take_bt_probe_stats().0;
            assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip {id} L{lvl}");
            let y=z.len() as i64;
            let d=100.0*(y-x) as f64/x as f64;
            if d>w {w=d; wid=id;}
            a+=x; b+=y;
        }
        println!("L{lvl}: size {:>+8.4}%   bt probes {:>+7.2}%   worst {} {:+.4}%",
            100.0*(b-a) as f64/a as f64,
            if pa>0 {100.0*(pb as f64-pa as f64)/pa as f64} else {0.0},
            if wid.is_empty(){"none"}else{wid}, w);
    }
    rusty_zstd::set_bt_deep_min_arm(2.0);
}
