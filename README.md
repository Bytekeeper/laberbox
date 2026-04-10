# Laberbox

A lightweight comment server for static sites (Hugo, Zola, Jekyll, …). Visitors submit a form; the server creates a branch and opens a pull request against your site's GitHub repository. Merging the PR adds the comment to a `comments.yaml` file co-located with the post, and your next site build picks it up.

No database, no third-party service. Comments are stored as plain YAML in your repo and reviewed by you before they appear.

## How it works

1. Visitor fills in the comment form and submits it.
2. The server appends the comment to `{content_dir}/{post-path}/comments.yaml` on a new branch and opens a PR.
3. You review and merge the PR.
4. Your CI/CD rebuilds the site; the comment appears.

---

## Setup

### Prerequisites

- A GitHub repository that holds your site's source (the one your SSG reads from).
- A [GitHub personal access token](https://github.com/settings/tokens) with the following permissions:
  - **Fine-grained token**: *Contents* (read & write) and *Pull requests* (read & write) on the target repository.
  - **Classic token**: `repo` scope.

### Building

```sh
cargo build --release
# binary is at target/release/laberbox
```

### Configuration

Create `config.yaml` next to the binary:

```yaml
# Address and port to listen on.
# Bind to 127.0.0.1 and put a reverse proxy in front (recommended).
listen: "127.0.0.1:3000"

# GitHub personal access token.
token: "github_pat_..."

# GitHub repository to open pull requests against.
owner: "your-github-username"
repo:  "your-site-repo"

# Root content directory inside the repository.
# Hugo / Zola: "content"   Jekyll: "."  or "_posts" depending on your layout.
content_dir: "content"

# Identity used for the automated commits.
committer:
  name: "Comment Bot"
  email: "comments@example.com"
```

### Running

```sh
./laberbox
```

Logs go to stderr. The default log level is `INFO`; set `RUST_LOG=debug` for verbose output.

---

## Running behind a reverse proxy (recommended)

Exposing the server directly to the internet works, but a reverse proxy in front gives you TLS, better DDoS protection, and lets you keep the server bound to `127.0.0.1`.

### Caddy

```caddyfile
comments.example.com {
    reverse_proxy 127.0.0.1:3000
}
```

Caddy handles TLS automatically via Let's Encrypt.

### nginx

```nginx
server {
    listen 443 ssl;
    server_name comments.example.com;

    ssl_certificate     /etc/letsencrypt/live/comments.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/comments.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:3000;
    }
}
```

Use [Certbot](https://certbot.eff.org/) or similar to obtain the certificate.

---

## Zola integration

### Requirements

- Zola **0.14** or later (`load_data` with `required = false`).
- Posts must be accessible by URL path — the standard Zola setup works without changes.

### 1. Copy the shortcode

Copy `integrations/zola/templates/shortcodes/comments.html` from this repository into your Zola site at:

```
templates/shortcodes/comments.html
```

### 2. Configure your site

Add the server URL to `config.toml`:

```toml
base_url = "https://example.com"   # must be set correctly — used to build redirect_url

[extra]
comment_server_url = "https://comments.example.com"
```

`base_url` is already required by Zola; just make sure it matches your production URL so that the redirect after form submission lands on the right page.

### 3. Add comments to a page

In any page's Markdown front matter there is nothing extra to do. Place the shortcode where you want comments to appear:

```markdown
{{ comments() }}
```

To enable comments on **all** posts automatically, add this to the bottom of `templates/page.html` instead:

```html
{% include "shortcodes/comments.html" %}
```

### How comments are stored

When a PR is merged, the comment is written to:

```
content/{post-path}/comments.yaml
```

For example, a post at `https://example.com/blog/my-post/` gets its comments at `content/blog/my-post/comments.yaml`. The shortcode loads that file at build time using `load_data`; if it does not exist yet the form is still shown with no error.

> **Note:** Zola copies non-Markdown files from content directories to the output, so `comments.yaml` will be publicly served alongside the post. This is not a security concern since comments are public, but be aware the raw data is accessible.

### Triggering a rebuild after merge

Laberbox itself must run on a host that supports long-running processes — a VPS (Hetzner, DigitalOcean, Linode, …), a home server, or a container platform. Static hosting services like Netlify, GitHub Pages, or Cloudflare Pages cannot run it.

Your *site*, however, can be hosted anywhere. To have comments appear automatically after you merge a PR, configure your static host or a GitHub Actions workflow to rebuild on pushes to the default branch — the PR merge triggers it. Most Zola deployment guides already cover this step.
