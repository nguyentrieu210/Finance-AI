//! Bounded Poppler rasterization. Run the worker inside an isolated parser
//! service with memory, CPU, process and network limits configured by operator.
use super::{Rotation,RunFailure};
use sha2::{Digest,Sha256};
use std::{env,fs,path::{Path,PathBuf},process::Stdio,time::{Duration,SystemTime,UNIX_EPOCH}};
use tokio::{io::AsyncReadExt,process::Command,time};

#[derive(Clone)]
pub(super) struct Config { info:PathBuf, render:PathBuf, fingerprint:[u8;32] }
impl Config {
    pub(super) fn from_env()->Result<Option<Self>,&'static str>{
        let Some(info)=env::var_os("FINANCE_PDFINFO").map(PathBuf::from) else{return Ok(None)};
        let render=env::var_os("FINANCE_PDFTOPPM").map(PathBuf::from).ok_or("PDF renderer missing")?;
        let mut hash=Sha256::new();
        for path in [&info,&render]{
            if !path.is_absolute(){return Err("PDF binary path must be absolute")}
            let metadata=fs::metadata(path).map_err(|_|"PDF binary missing")?;
            if !metadata.is_file()||metadata.len()==0||metadata.len()>100*1024*1024{return Err("PDF binary size invalid")}
            hash.update(Sha256::digest(fs::read(path).map_err(|_|"PDF binary unreadable")?));
        }
        hash.update(b"poppler/png/scale-to-2400/v1");
        Ok(Some(Self{info,render,fingerprint:hash.finalize().into()}))
    }
    pub(super) fn fingerprint(&self)->[u8;32]{self.fingerprint}

    pub(super) async fn open(&self,bytes:&[u8])->Result<Document,RunFailure>{
        if bytes.len()<5||bytes.len()>25*1024*1024||!bytes.starts_with(b"%PDF-"){return Err(RunFailure::Permanent)}
        let file=PrivatePdf::new(bytes)?;
        let info=command(&self.info,&["-enc","UTF-8"],&file.path,65536).await?;
        let info=std::str::from_utf8(&info).map_err(|_|RunFailure::Permanent)?;
        let count=parse_count(info)?;
        let end=count.to_string();
        let detailed=command(&self.info,&["-f","1","-l",&end],&file.path,262144).await?;
        let detailed=std::str::from_utf8(&detailed).map_err(|_|RunFailure::Permanent)?;
        let rotations=parse_rotations(detailed,count)?;
        Ok(Document{file,config:self.clone(),rotations})
    }
}

struct PrivatePdf{dir:PathBuf,path:PathBuf}
impl PrivatePdf{
    fn new(bytes:&[u8])->Result<Self,RunFailure>{
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_|RunFailure::Transient)?.as_nanos();
        let dir=env::temp_dir().join(format!("finance-pdf-{}-{nonce}",std::process::id()));
        let mut builder=fs::DirBuilder::new();
        #[cfg(unix)] {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir).map_err(|_|RunFailure::Transient)?;
        let file=Self{path:dir.join("source.pdf"),dir};
        let mut options=fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]{
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut output=options.open(&file.path).map_err(|_|RunFailure::Transient)?;
        output.write_all(bytes).map_err(|_|RunFailure::Transient)?;
        Ok(file)
    }
}
impl Drop for PrivatePdf{fn drop(&mut self){let _=fs::remove_file(&self.path);let _=fs::remove_dir(&self.dir);}}

pub(super) struct Document{file:PrivatePdf,config:Config,rotations:Vec<Rotation>}
impl Document{
    pub(super) fn count(&self)->u32{self.rotations.len() as u32}
    pub(super) async fn render(&self,page:u32)->Result<(Vec<u8>,Rotation),RunFailure>{
        let index=page.checked_sub(1).ok_or(RunFailure::Permanent)? as usize;
        let rotation=*self.rotations.get(index).ok_or(RunFailure::Permanent)?;
        let page=page.to_string();
        let bytes=command(&self.config.render,&["-f",&page,"-l",&page,"-scale-to","2400","-png","-singlefile"],&self.file.path,25*1024*1024).await?;
        Ok((bytes,rotation))
    }
}
async fn command(binary:&Path,args:&[&str],file:&Path,max:u64)->Result<Vec<u8>,RunFailure>{
    let mut child=Command::new(binary).env_clear().env("PATH","/usr/bin:/bin").env("LC_ALL","C")
        .args(args).arg(file).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true)
        .spawn().map_err(|_|RunFailure::Permanent)?;
    let mut stdout=child.stdout.take().ok_or(RunFailure::Permanent)?.take(max+1);
    let io=async{
        let mut bytes=Vec::new();
        stdout.read_to_end(&mut bytes).await.map_err(|_|RunFailure::Transient)?;
        if bytes.len() as u64>max{return Err(RunFailure::Permanent)}
        if !child.wait().await.map_err(|_|RunFailure::Transient)?.success(){return Err(RunFailure::Permanent)}
        Ok(bytes)
    };
    time::timeout(Duration::from_secs(45),io).await.map_err(|_|RunFailure::Transient)?
}
fn parse_count(text:&str)->Result<u32,RunFailure>{
    let mut values=text.lines().filter_map(|line|line.strip_prefix("Pages:"));
    let count=values.next().and_then(|s|s.trim().parse::<u32>().ok()).ok_or(RunFailure::Permanent)?;
    if values.next().is_some()||!(1..=200).contains(&count){return Err(RunFailure::Permanent)}
    Ok(count)
}
fn parse_rotations(text:&str,count:u32)->Result<Vec<Rotation>,RunFailure>{
    let mut rotations=vec![None;count as usize];
    for line in text.lines(){
        let Some((label,value))=line.split_once(':')else{continue};
        let words:Vec<_>=label.split_whitespace().collect();
        let page=match words.as_slice(){
            ["Page","rot"] if count==1=>1,
            ["Page",number,"rot"]=>number.parse::<u32>().map_err(|_|RunFailure::Permanent)?,
            _=>continue,
        };
        let rotation=match value.trim(){"0"=>Rotation::R0,"90"=>Rotation::R90,"180"=>Rotation::R180,"270"=>Rotation::R270,_=>return Err(RunFailure::Permanent)};
        let slot=rotations.get_mut(page.checked_sub(1).ok_or(RunFailure::Permanent)? as usize).ok_or(RunFailure::Permanent)?;
        if slot.replace(rotation).is_some(){return Err(RunFailure::Permanent)}
    }
    rotations.into_iter().map(|rotation|rotation.ok_or(RunFailure::Permanent)).collect()
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn refuses_incomplete_or_excessive_metadata(){
        assert_eq!(parse_count("Pages: 2"),Ok(2));
        assert!(parse_count("Pages: 201").is_err());
        assert_eq!(parse_rotations("Page 1 rot: 90\nPage 2 rot: 0",2),Ok(vec![Rotation::R90,Rotation::R0]));
        assert!(parse_rotations("Page 1 rot: 90",2).is_err());
    }
}
