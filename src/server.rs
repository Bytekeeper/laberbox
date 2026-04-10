use crate::{CONFIG, Comment, Post};
use anyhow::Context;
use bytes::Bytes;
use core::convert::Infallible;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use log::{debug, info};
use octocrab::models::repos::{CommitAuthor, ContentItems, Object};
use octocrab::params::repos::Reference;
use rand::RngExt;
use rand::rng;
use std::fmt::Write;
use std::time::Duration;
use std::time::SystemTime;
use tokio::net::TcpListener;
use tower::ServiceBuilder;

fn error_response(status: StatusCode, msg: &'static str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::from(msg)))
        .unwrap()
}

async fn post_comment_service(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    // Prevent crashing the service by simple DDOS attacks
    let body = Limited::new(req.into_body(), 100 * 1024);
    let Ok(bytes) = body.collect().await.map(|c| c.to_bytes()) else {
        return Ok(error_response(StatusCode::BAD_REQUEST, "Comment size limit exceeded"));
    };
    let Ok(post): Result<Post, _> = serde_urlencoded::from_bytes(&bytes) else {
        return Ok(error_response(StatusCode::BAD_REQUEST, "Invalid form data"));
    };

    if !post.url.is_empty()
        && !post.url.starts_with("https://")
        && !post.url.starts_with("http://")
    {
        return Ok(error_response(StatusCode::BAD_REQUEST, "URL must be http or https"));
    }

    let time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let comment_id = format!("{}_{}", time, rng().random_range(0..999999999u64));

    let comment = Comment {
        id: &comment_id,
        message: &post.message,
        name: &post.name,
        url: &post.url,
        date: time,
    };

    let oc = octocrab::instance();
    let branch_name = format!("comments/{}", comment_id);
    let config = CONFIG.get().unwrap();
    let path = format!(
        "{}/{}/comments.yaml",
        config.content_dir.trim_matches('/'),
        post.path.trim_matches('/')
    );

    let repo = oc.repos(&config.owner, &config.repo);

    let master_sha = match repo
        .get_ref(&Reference::Branch("master".to_string()))
        .await
        .expect("Could not get master ref")
        .object
    {
        Object::Commit { sha, .. } | Object::Tag { sha, .. } => sha,
        _ => unreachable!(),
    };

    debug!("Creating branch {} from {}", branch_name, master_sha);
    repo.create_ref(&Reference::Branch(branch_name.clone()), master_sha)
        .await
        .expect("Could not create branch");

    debug!("Requesting {}", path);
    let content_items = match repo.get_content().path(&path).send().await {
        Ok(content_items) => content_items,
        Err(_) => {
            info!("Assuming no comments present yet at {}", path);
            ContentItems { items: Vec::new() }
        }
    };

    assert!(content_items.items.len() <= 1);
    let content = content_items.items.first();
    let new_comment =
        serde_yaml::to_string(&[&comment]).expect("Could not convert comment to yaml");
    let author = CommitAuthor {
        name: config.committer.name.clone(),
        email: Some(config.committer.email.clone()),
        date: None,
    };

    if let Some(content) = content {
        // GitHub API requires the SHA of the old file to update it
        let (mut content, sha) = (content.decoded_content().unwrap(), content.sha.clone());
        writeln!(&mut content, "{}", new_comment).expect("Could not add comment to file");
        debug!("Found existing file at {} with sha {}", path, sha);
        repo.update_file(
            &path,
            format!("Added comment from '{}'", comment.name),
            content,
            sha,
        )
        .branch(&branch_name)
        .commiter(author)
        .send()
        .await
        .expect("Could not update file");
    } else {
        debug!("Creating new file at {}", path);
        repo.create_file(
            &path,
            format!("Added comment from '{}'", comment.name),
            new_comment,
        )
        .branch(&branch_name)
        .commiter(author)
        .send()
        .await
        .expect("Could not create file");
    }

    oc.pulls(&config.owner, &config.repo)
        .create(
            format!("New comment from {}", comment.name),
            branch_name,
            "master",
        )
        .send()
        .await
        .expect("Could not create PR");
    info!("New comment from {} added", comment.name);

    Ok(Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header("Location", post.redirect_url)
        .body(Full::new(Bytes::new()))
        .unwrap())
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    octocrab::initialise(
        octocrab::Octocrab::builder()
            .personal_token(CONFIG.get().unwrap().token.clone())
            .build()?,
    );

    let addr = CONFIG.get().context("Missing config")?.listen;
    info!("Listening on {}", addr);

    let svc = ServiceBuilder::new()
        .buffer(100)
        .rate_limit(1, Duration::from_secs(10))
        .service(tower::service_fn(post_comment_service));

    let listener = TcpListener::bind(addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let svc = TowerToHyperService::new(svc.clone());
        tokio::spawn(async move {
            if let Err(err) = Builder::new(TokioExecutor::new())
                .serve_connection(io, svc)
                .await
            {
                log::error!("Error serving connection: {:?}", err);
            }
        });
    }
}
