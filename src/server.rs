use crate::{Comment, Post, CONFIG};
use core::convert::Infallible;
use http_body::Limited;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use log::{debug, info};
use octocrab::models::repos::{CommitAuthor, ContentItems, Object};
use octocrab::params::repos::Reference;
use rand::{thread_rng, Rng};
use std::fmt::Write;
use std::net::SocketAddr;
use std::time::Duration;
use std::time::SystemTime;
use tower::limit::{rate::Rate, RateLimit};

async fn post_comment_service(req: Request<Body>) -> hyper::Result<Response<Body>> {
    if req.method() == hyper::Method::OPTIONS {
        return Ok(Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(ACCESS_CONTROL_ALLOW_METHODS, "OPTIONS, POST")
            .header(ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(ACCESS_CONTROL_ALLOW_HEADERS, "content-type")
            .body("".into())
            .unwrap());
    }
    // Prevent crashing the service by simple DDOS attacks
    let body = Limited::new(req.into_body(), 100 * 1024);
    let Ok(post_request) = hyper::body::to_bytes(body).await else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body("Comment size limit exceeded".into())
            .unwrap());
    };
    let Ok(post): Result<Post, _> = serde_json::from_slice(&*post_request) else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body("Invalid JSON".into())
            .unwrap());
    };

    let time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let comment_id = format!("{}_{}", time, thread_rng().gen_range(0..999999999));

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
    let path = format!("content{}comments.yaml", post.path);

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
    // There can't be more than one file with the same name:
    assert!(content_items.items.len() <= 1);
    let content = content_items.items.iter().next();
    let new_comment =
        serde_yaml::to_string(&[&comment]).expect("Could not convert comment to yaml");
    let author = CommitAuthor {
        name: "Comment0r".to_string(),
        email: "none@example.com".to_string(),
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
        .status(hyper::StatusCode::CREATED)
        .header(ACCESS_CONTROL_ALLOW_METHODS, "OPTIONS, POST")
        .header(ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(ACCESS_CONTROL_ALLOW_HEADERS, "content-type")
        .body("That worked, now goeth forth".into())
        .unwrap())
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    octocrab::initialise(
        octocrab::Octocrab::builder()
            .personal_token(CONFIG.get().unwrap().token.clone())
            .build()?,
    );

    let addr = SocketAddr::from(CONFIG.get().unwrap().listen);
    info!("Listening on {}", addr);

    let post_comment_service = make_service_fn(|_conn| async {
        Ok::<_, Infallible>(RateLimit::new(
            service_fn(post_comment_service),
            Rate::new(1, Duration::from_secs(10)),
        ))
    });
    let server = Server::bind(&addr).serve(post_comment_service);
    Ok(server.await?)
}
