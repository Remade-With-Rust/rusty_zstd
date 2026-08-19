//! GATE 14 @ L3 shipped state: probes and size against the pre-gate behaviour.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    for lvl in [3i32,4]{
        let (mut a,mut b,mut pa,mut pb)=(0i64,0i64,0u64,0u64);
        let (mut raised,mut held)=(0,0);
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            // pre-gate: cut pinned at 8, cand2 at 8
            rusty_zstd::set_dfast_good_ml_arm(8);
            rusty_zstd::set_dfast_good_ml2_arm(8);
            rusty_zstd::set_nl_off_worse_arm(-1.0);
            let _=rusty_zstd::take_mm();
            let x=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            pa+=rusty_zstd::take_mm().0;
            // shipped
            rusty_zstd::set_dfast_good_ml_arm(0);
            rusty_zstd::set_dfast_good_ml2_arm(0);
            rusty_zstd::set_nl_off_worse_arm(0.60);
            let _=rusty_zstd::take_mm();
            let z=rusty_zstd::compress(src,lvl).unwrap();
            pb+=rusty_zstd::take_mm().0;
            assert_eq!(rusty_zstd::decompress(&z).unwrap(), src);
            let y=z.len() as i64;
            if y<x {raised+=1;} else if y>x {held+=1;}
            a+=x; b+=y;
        }
        println!("L{lvl}: size {:>+8.4}%   probes {:>+7.2}%   {raised} corpora smaller, {held} larger",
            100.0*(b-a) as f64/a as f64, 100.0*(pb as f64-pa as f64)/pa as f64);
    }
    rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
    rusty_zstd::set_nl_off_worse_arm(0.60);
}
