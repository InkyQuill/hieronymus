
use std::{io::{Read, Write}, net::TcpListener, sync::{Arc,atomic::{AtomicUsize,Ordering}},time::{Duration,Instant}};
fn server(status: u16, body: String) -> (String, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
 let listener=TcpListener::bind("127.0.0.1:0").unwrap();
 let origin=format!("http://{}",listener.local_addr().unwrap());
 listener.set_nonblocking(true).unwrap();
 let count=Arc::new(AtomicUsize::new(0)); let c=count.clone(); let url=origin.clone();
 let worker=std::thread::spawn(move || {
  let start=Instant::now();
  while start.elapsed()<Duration::from_millis(650) {
   match listener.accept() {
    Ok((mut stream,_))=>{
     stream.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
     let mut input=Vec::new(); let mut buf=[0;4096];
     loop { match stream.read(&mut buf) { Ok(0)|Err(_)=>break, Ok(n)=>{input.extend_from_slice(&buf[..n]);if let Some(pos)=input.windows(4).position(|w|w==b"\r\n\r\n") {
      let headers=String::from_utf8_lossy(&input[..pos]);let len=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0);
      if input.len()>=pos+4+len {break;}
     }}}}
     let attempt=c.fetch_add(1,Ordering::SeqCst);
     let actual=if attempt==0 {status}else{200};
     let location=if actual==307 {format!("Location: {url}/redirected\r\n")}else{String::new()};
     let wire=format!("HTTP/1.1 {actual} Audit\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{location}\r\n{body}",body.len());
     let _=stream.write_all(wire.as_bytes());
    }, Err(_)=>std::thread::sleep(Duration::from_millis(5))
   }
  }
 });
 (origin,count,worker)
}
fn valid() -> String {r#"{"model":"audit","usage":{},"answers":{"q":{"type":"noul","noul":0.8}}}"#.into()}
#[tokio::test]
async fn audit_rejects_out_of_range_noul() {
 let (url,_,worker)=server(200,valid().replace("0.8","1.5"));let result=ask(&url).await;worker.join().unwrap();
 assert!(result.is_err(),"accepted probability 1.5; domain validation required");
}
#[tokio::test]
async fn audit_rejects_invalid_json() {
 let (url,_,worker)=server(200,"{broken".into());let result=ask(&url).await;worker.join().unwrap();
 assert!(result.is_err());
}
#[tokio::test]
async fn audit_one_bad_answer_rejects_typed_batch() {
 let body=r#"{"model":"audit","usage":{},"answers":{"q":{"type":"noul","noul":0.8},"bad":{"type":"noul","noul":"invalid"}}}"#;
 let (url,_,worker)=server(200,body.into());let result=ask(&url).await;worker.join().unwrap();
 assert!(result.is_err(),"decoder silently accepted malformed answer");
}
#[tokio::test]
async fn audit_rejects_oversized_response() {
 let mut body=valid();body.pop();body.push_str(&format!(",\"padding\":\"{}\"}}","x".repeat(70000)));
 let (url,_,worker)=server(200,body);let result=ask(&url).await;worker.join().unwrap();
 assert!(result.is_err(),"accepted response larger than Hieronymus 64 KiB limit");
}
#[tokio::test]
async fn audit_does_not_follow_redirects() {
 let (url,count,worker)=server(307,valid());let _=ask(&url).await;worker.join().unwrap();
 assert_eq!(count.load(Ordering::SeqCst),1,"followed redirect despite zero retries");
}
#[tokio::test]
async fn audit_no_retry_on_429() {
 let (url,count,worker)=server(429,valid());let result=ask(&url).await;worker.join().unwrap();
 assert!(result.is_err());assert_eq!(count.load(Ordering::SeqCst),1);
}
#[tokio::test]
async fn audit_accepts_named_noul() {
 let (url,_,worker)=server(200,valid());let result=ask(&url).await;worker.join().unwrap();
 assert!(result.is_ok(),"{result:?}");
}

async fn ask(url:&str)->Result<(),String>{
 use typesafe_sdk::{TypeSafeClient,Noul,RetryPolicy};
 let c=TypeSafeClient::builder().api_key("fixture").base_url(url).model("audit").retry(RetryPolicy::default().with_max_retries(0)).timeout(Duration::from_millis(300)).build().map_err(|e|format!("{e:?}"))?;
 c.system_one().state("fixture").question("q",Noul::new().instructions("?")).send().await.map(|_|()).map_err(|e|format!("{e:?}"))
}
