//! The search-strength shift `((ip - anchor) >> N)`, hardcoded at 8 and never
//! gated. It is what produces the positions/byte spread. Unlike the back-fill
//! writes of 4.40, positions are DEPENDENT work on the critical path.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("shift   {:>12}{:>13}{:>11}{:>11}   worst corpus","size","positions","size %","pos %");
    let (mut bs,mut bp)=(0i64,0u64);
    let mut base=vec![];
    for n in [8u32,7,6,5,9,10]{
        let (mut sz,mut mm)=(0i64,0u64);
        let mut per=vec![];
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_accel_shift_arm(n);
            let _=rusty_zstd::take_mm();
            let z=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            sz+=z; per.push(z);
            mm+=rusty_zstd::take_mm().0;
        }
        if n==8 {bs=sz; bp=mm; base=per.clone();
            println!("{n:>5}   {sz:>12}{mm:>13}{:>11}{:>11}   (today)","-","-");
        } else {
            let (mut w,mut wid)=(0f64,"");
            for (k,id) in IDS.iter().enumerate(){
                if k<per.len(){let d=100.0*(per[k]-base[k]) as f64/base[k] as f64;
                    if d>w {w=d; wid=id;}}
            }
            println!("{n:>5}   {sz:>12}{mm:>13}{:>+10.4}%{:>+10.2}%   {wid} {w:+.3}%",
                100.0*(sz-bs) as f64/bs as f64, 100.0*(mm as f64-bp as f64)/bp as f64);
        }
    }
    rusty_zstd::set_accel_shift_arm(8);
}
