# Cellular viewer

A static site that reads a `.cellexport` file produced by the runner and draws
the project's code composition as a squarified treemap, commit by commit.

```sh
pnpm install
pnpm run dev      # http://localhost:5173
pnpm run build    # static site in dist/
pnpm run preview  # serve the build on http://localhost:8080
```

Open an index with **File → Open…**, or drop a `.cellexport` file anywhere on
the window. A hosted index can be opened directly with `?src=<url>`, which is
how a deployment can link to a published index.

Nothing in `dist/` is committed; deployments build it as an artifact. Asset
paths are relative, so the same build works from a domain root (Cloudflare
Pages) and from a repository subpath (GitHub Pages). Set `VITE_BASE` if a
deployment needs an absolute prefix.
