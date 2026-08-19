//! GATE 12 @ L3, re-opened. The stride knob read DEAD because it controls
//! `find_lazy`'s span walk, which L3 never enters. DFast's OWN back-fill is two
//! positions per match. Two questions, both never asked:
//!   (a) ANCHOR: the short fill uses `best_ip`, the long fill uses `ip`.
//!   (b) DENSITY: is two positions per match the right number?
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("=== (a) ANCHOR: long fill on `ip` (today) vs `best_ip` (C-consistent), L{lvl} ===");
    println!("{:<14}{:>10}{:>10}{:>10}","corpus","today","C-anchor","delta");
    let (mut ta,mut tb)=(0i64,0i64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_fill_stride_arm(0);
        rusty_zstd::set_dfast_fill_anchor_arm(false);
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        rusty_zstd::set_dfast_fill_anchor_arm(true);
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        ta+=a; tb+=b;
        let mark=if b<a {"  <- C smaller"} else if b>a {"  <- C bigger"} else {""};
        println!("{id:<14}{a:>10}{b:>10}{:>+10}{mark}",b-a);
    }
    println!("{:<14}{ta:>10}{tb:>10}{:>+10}  ({:+.4}%)","TOTAL",tb-ta,100.0*(tb-ta) as f64/ta as f64);

    println!("\n=== (b) DENSITY: interior back-fill stride, L{lvl} ===");
    rusty_zstd::set_dfast_fill_anchor_arm(false);
    let mut base=vec![];
    for s in [0usize,16,8,4,2,1]{
        let (mut tot,mut ins,mut worst,mut wid)=(0i64,0u64,0f64,"");
        let mut sizes=vec![];
        for (k,id) in IDS.iter().enumerate(){
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_dfast_fill_stride_arm(s);
            let _=rusty_zstd::take_dfast_fill();
            let sz=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            ins+=rusty_zstd::take_dfast_fill();
            tot+=sz; sizes.push(sz);
            if s!=0 {
                let d=100.0*(sz-base[k]) as f64/base[k] as f64;
                if d>worst {worst=d; wid=id;}
            }
        }
        if s==0 {base=sizes.clone(); println!("stride {s:>3} (today) total {tot:>10}  interior inserts {ins:>10}");}
        else {
            let bt:i64=base.iter().sum();
            println!("stride {s:>3}          total {tot:>10} ({:>+7.4}%)  interior inserts {ins:>10}  worst {:>+6.3}% ({wid})",
                100.0*(tot-bt) as f64/bt as f64, worst);
        }
    }
    rusty_zstd::set_dfast_fill_stride_arm(0);
}
