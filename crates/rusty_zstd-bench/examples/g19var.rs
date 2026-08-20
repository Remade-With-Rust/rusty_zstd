//! Can a FINER cut than "ladder off" keep samba? Above 2 MiB, compare:
//!   A  ladder fully off            (4.77 as shipped)
//!   B  raw-escape off, drift kept  (ratio 2.0, drift 2.00)
//!   C  raw-escape off, drift eased (ratio 2.0, drift 0.30)
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    for mb in [4usize,8]{
        println!("\n===== {mb} MiB =====");
        println!("{:<14}{:>11}{:>11}{:>11}","corpus","A off","B no-rawesc","C drift 0.30");
        let mut tot=[(0i64,0i64,0i64,0i64);1];
        let mut worst=[(0.0f64,String::new()),(0.0,String::new()),(0.0,String::new())];
        for id in IDS{
            let Some(f)=load(id) else{continue};
            if f.len()<(mb<<20) {continue;}
            let src=&f[..mb<<20];
            rusty_zstd::set_g5_fast_len_arm(usize::MAX);
            rusty_zstd::set_g5_fast_arms(2.00,0.70,2.00);
            let base=rusty_zstd::compress(src,1).unwrap().len() as i64;
            rusty_zstd::set_g5_fast_len_arm(0);
            rusty_zstd::set_g5_fast_arms(2.00,0.70,2.00);
            let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
            rusty_zstd::set_g5_fast_len_arm(usize::MAX);
            rusty_zstd::set_g5_fast_arms(2.00,2.00,2.00);
            let b=rusty_zstd::compress(src,1).unwrap().len() as i64;
            rusty_zstd::set_g5_fast_arms(2.00,2.00,0.30);
            let c=rusty_zstd::compress(src,1).unwrap().len() as i64;
            tot[0].0+=base; tot[0].1+=a; tot[0].2+=b; tot[0].3+=c;
            let d=|x:i64|100.0*(x-base) as f64/base as f64;
            for (i,v) in [d(a),d(b),d(c)].iter().enumerate(){
                if *v>worst[i].0 {worst[i]=(*v,id.to_string());}
            }
            if a!=base||b!=base||c!=base{
                println!("{id:<14}{:>10.3}%{:>10.3}%{:>10.3}%",d(a),d(b),d(c));
            }
        }
        let (base,a,b,c)=tot[0];
        println!("{:<14}{:>10.4}%{:>10.4}%{:>10.4}%","TOTAL",
            100.0*(a-base) as f64/base as f64,
            100.0*(b-base) as f64/base as f64,
            100.0*(c-base) as f64/base as f64);
        println!("worst:  A {} {:+.3}%   B {} {:+.3}%   C {} {:+.3}%",
            worst[0].1,worst[0].0,worst[1].1,worst[1].0,worst[2].1,worst[2].0);
    }
    rusty_zstd::set_g5_fast_len_arm(0);
    rusty_zstd::set_g5_fast_arms(2.00,0.70,2.00);
}
