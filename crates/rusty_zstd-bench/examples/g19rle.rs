//! Is there an RLE threshold that separates versions-16m (r_prev 0.0028, wants
//! splitting, -3.935%) from TRUE RLE (zeros/text, which +594%/+664% if split)?
const IDS:&[&str]=&["zeros-32m","text-32m","versions-16m","incomp-32m","jsonlog-16m","smallmsg-8m","nci","xml"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("r_prev for the degenerate corpora -- can one threshold separate them?\n");
    println!("{:<14}{:>12}{:>14}{:>14}","corpus","mean r_prev","split @96KB","verdict");
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        unsafe{ std::env::remove_var("RZSTD_BLOCK_KB"); }
        let _=rusty_zstd::take_g5_inputs();
        let a=rusty_zstd::compress(src,1).unwrap().len() as f64;
        let (rp,_)=rusty_zstd::take_g5_inputs();
        unsafe{ std::env::set_var("RZSTD_BLOCK_KB","96"); }
        let b=rusty_zstd::compress(src,1).unwrap().len() as f64;
        unsafe{ std::env::remove_var("RZSTD_BLOCK_KB"); }
        let d=100.0*(b-a)/a;
        println!("{id:<14}{rp:>12.6}{d:>13.3}%{:>14}",
            if d < -0.05 {"WANTS SPLIT"} else if d > 0.5 {"MUST NOT"} else {"neutral"});
    }
    println!("\nG5_RLE_MAX is currently 0.01 -- anything below it returns base unsplit");
}
