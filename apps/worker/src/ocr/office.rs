//! Converts only admitted Office containers using a dedicated isolated renderer.
use super::RunFailure;
use sha2::{Digest,Sha256};
use std::{env,fs,path::PathBuf,process::Stdio,time::Duration};
use tokio::{io::{AsyncReadExt,AsyncWriteExt},process::Command,time};

pub(super) const DOCX:&str="application/vnd.openxmlformats-officedocument.wordprocessingml.document";
pub(super) const XLSX:&str="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const LIMIT:usize=25*1024*1024;
#[derive(Clone)]
pub(super) struct Config{adapter:PathBuf,converter:PathBuf,digest:[u8;32]}
impl Config{
 pub(super) fn from_env()->Result<Option<Self>,&'static str>{
  let Some(adapter)=env::var_os("FINANCE_OFFICE_INGEST_ADAPTER").map(PathBuf::from)else{return Ok(None)};
  let converter=env::var_os("FINANCE_OFFICE_LIBREOFFICE").map(PathBuf::from).ok_or("Office renderer missing")?;
  let mut hash=Sha256::new();
  for path in [&adapter,&converter]{
   if !path.is_absolute(){return Err("Office renderer path must be absolute")}
   let m=fs::metadata(path).map_err(|_|"Office renderer missing")?;
   if !m.is_file()||m.len()==0||m.len()>100*1024*1024{return Err("Office renderer invalid")}
   hash.update(Sha256::digest(fs::read(path).map_err(|_|"Office renderer unreadable")?));
  }
  hash.update(b"finance.office.render/v1");
  Ok(Some(Self{adapter,converter,digest:hash.finalize().into()}))
 }
 pub(super) fn fingerprint(&self)->[u8;32]{self.digest}
 pub(super) async fn render(&self,source:&[u8],mime:&str)->Result<Vec<u8>,RunFailure>{
  if source.is_empty()||source.len()>LIMIT||!source.starts_with(b"PK\x03\x04"){return Err(RunFailure::Permanent)}
  let format=match mime{DOCX=>"docx",XLSX=>"xlsx",_=>return Err(RunFailure::Permanent)};
  let mut child=Command::new(&self.adapter).env_clear().env("PATH","/usr/bin:/bin").arg(format).arg(&self.converter)
   .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true)
   .spawn().map_err(|_|RunFailure::Permanent)?;
  let mut stdin=child.stdin.take().ok_or(RunFailure::Permanent)?;
  let mut stdout=child.stdout.take().ok_or(RunFailure::Permanent)?.take((LIMIT+1) as u64);
  let operation=async{
   let write=async{stdin.write_all(source).await.map_err(|_|RunFailure::Transient)?;stdin.shutdown().await.map_err(|_|RunFailure::Transient)};
   let read=async{let mut bytes=Vec::new();stdout.read_to_end(&mut bytes).await.map_err(|_|RunFailure::Transient)?;if bytes.len()>LIMIT||!bytes.starts_with(b"%PDF-"){return Err(RunFailure::Permanent)}Ok(bytes)};
   let (_,bytes)=tokio::try_join!(write,read)?;
   if !child.wait().await.map_err(|_|RunFailure::Transient)?.success(){return Err(RunFailure::Permanent)}
   Ok(bytes)
  };
  time::timeout(Duration::from_secs(120),operation).await.map_err(|_|RunFailure::Transient)?
 }
}

pub(super) fn jpeg_dimensions(bytes:&[u8])->Result<(u32,u32),RunFailure>{
 if bytes.len()<4||bytes.len()>LIMIT||!bytes.starts_with(&[0xff,0xd8]){return Err(RunFailure::Permanent)}
 let mut i=2usize;
 while i<bytes.len(){
  if bytes[i]!=0xff{return Err(RunFailure::Permanent)}
  while i<bytes.len()&&bytes[i]==0xff{i+=1}
  let marker=*bytes.get(i).ok_or(RunFailure::Permanent)?;i+=1;
  if marker==0xda||marker==0xd9||marker==0x00{return Err(RunFailure::Permanent)}
  if marker==0x01||(0xd0..=0xd7).contains(&marker){continue}
  let len=u16::from_be_bytes(bytes.get(i..i+2).ok_or(RunFailure::Permanent)?.try_into().map_err(|_|RunFailure::Permanent)?) as usize;
  if len<2||i+len>bytes.len(){return Err(RunFailure::Permanent)}
  if matches!(marker,0xc0|0xc1|0xc2|0xc3|0xc5|0xc6|0xc7|0xc9|0xca|0xcb|0xcd|0xce|0xcf){
   if len<8{return Err(RunFailure::Permanent)}
   let h=u16::from_be_bytes([bytes[i+3],bytes[i+4]]) as u32;
   let w=u16::from_be_bytes([bytes[i+5],bytes[i+6]]) as u32;
   if !(1..=8192).contains(&w)||!(1..=8192).contains(&h)||u64::from(w)*u64::from(h)>25_000_000{return Err(RunFailure::Permanent)}
   return Ok((w,h))
  }
  i+=len;
 }
 Err(RunFailure::Permanent)
}
#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn bounded_jpeg_headers(){
  let image=[0xff,0xd8,0xff,0xc0,0,8,8,0,32,0,64,1];
  assert_eq!(jpeg_dimensions(&image),Ok((64,32)));
  assert!(jpeg_dimensions(&image[..9]).is_err());
  let huge=[0xff,0xd8,0xff,0xc0,0,8,8,0xff,0xff,0xff,0xff,1];
  assert!(jpeg_dimensions(&huge).is_err());
 }
}
