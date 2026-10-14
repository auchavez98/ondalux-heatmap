# ondaluxheatmap.com

Static website for Ondalux Heatmap: English at `/`, Spanish at `/es/`.
No frameworks and no third-party requests (fonts, analytics or CDNs).

```
website/
  src/template.html   page template (both languages)
  src/en.json, es.json texts
  build.mjs           generates public/index.html, public/es/index.html,
                      sitemap.xml and copies ../dist/ondalux.exe to
                      public/downloads/ondalux-heatmap-<version>-windows-x64.exe
                      (size and SHA-256 go into the pages)
  public/             what is uploaded to public_html (assets, .htaccess, 404)
  serve.mjs           local preview: node website/serve.mjs  → http://localhost:4173
  deploy.ps1          build + upload over FTPS, or -Zip for cPanel
```

## Publish a new version

1. Build the app: `cargo build --release -p ondalux-app` and copy
   `target/release/ondalux.exe` to `dist/`.
2. Then either:
   - **Zip for cPanel**: `.\website\deploy.ps1 -Zip`, then in cPanel
     *File Manager → public_html → Upload* the zip and *Extract* it.
   - **FTP upload**: set `$env:ONDALUX_FTP_HOST` and `$env:ONDALUX_FTP_USER`
     (cPanel → FTP Accounts) and run `.\website\deploy.ps1`; the password is
     asked for at the prompt and never saved. Use `-RemoteDir ""` if the FTP
     account already opens in `public_html`.

Screenshots come from the app's UI test
(`ONDALUX_LANG=en|es ONDALUX_SHOTS=… cargo test -p ondalux-app ui_shots -- --ignored`)
and the sample reports from `cargo run -p ondalux-report --example report`.

After the first publish, enable SSL for the domain in cPanel (*SSL/TLS
Status → Run AutoSSL*); `.htaccess` redirects every request to
`https://ondaluxheatmap.com`.
