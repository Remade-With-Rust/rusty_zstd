//! GATE 14 @ L3, step 1: is it dead? The depth cut lives in `bt_find_best` and
//! `bt_depth_cut` additionally excludes non-opt strategies. L3 is DFast, which
//! allocates no chain and never calls the bt walk -- so the gate should be dead
//! TWICE over. Prove it on output AND on reachability.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    // reachability: does L3 call the bt walk at all?
    let (mut sp,mut rt)=(0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_bt_calls();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (a,b)=rusty_zstd::take_bt_calls(); sp+=a; rt+=b;
    }
    println!("L{lvl}: bt_find_best calls -- specialised {sp}, runtime {rt}");
    // output: does the depth arm move any byte?
    let mut moved=0; let mut n=0;
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let base=rusty_zstd::compress(src,lvl).unwrap();
        let mut any=false;
        for d in [1u32,2,4,8,64,512]{
            std::env::set_var("RZSTD_BT_DEPTH_TARGET", d.to_string());
            rusty_zstd::reset_env_arms();
            if rusty_zstd::compress(src,lvl).unwrap()!=base {any=true;}
        }
        std::env::remove_var("RZSTD_BT_DEPTH_TARGET");
        rusty_zstd::reset_env_arms();
        n+=1; if any {moved+=1;}
    }
    println!("L{lvl}: corpora whose output moves with the depth target: {moved}/{n}");
}
