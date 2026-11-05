# Migration to cytiq-engine

The GitHub repository, Cargo package, crate directory and GHCR image now use
`cytiq-engine`. The executable remains `cytiq`, including its arguments and output
formats. Library crate names and versions are unchanged.

## GitHub settings

Rename the existing repository in **Settings → General**; do not create a replacement
repository. The existing repository keeps its history and configuration. Update local
clones with:

```sh
git remote set-url origin git@github.com:cytiq/cytiq-engine.git
```

Keep the existing `main` ruleset and required checks. Workflow/job names are unchanged.
Keep the `crates-io` environment and its release-tag deployment policy. The release
workflows still use `GH_TOKEN`, `GPG_PRIVATE_KEY`, `GPG_PASSPHRASE` and
`GPG_FINGERPRINT`; renaming the repository does not require generating new credentials.
Check access if organization secrets, a GitHub App or a fine-grained token select
individual repositories.

GitHub redirects ordinary repository and Git URLs, but calls to hosted Actions do
not follow repository renames. All explicit repository checks and links in this
workspace now use the new name. See [GitHub's rename documentation](https://docs.github.com/en/repositories/creating-and-managing-repositories/renaming-a-repository).

## GHCR: copy existing images before switching consumers

The new image is `ghcr.io/cytiq/cytiq-engine`. A repository rename does not publish
images at this new registry path. Before merging or deploying the consumer changes,
copy the existing multi-platform image indexes, including their attestation manifests.
Do not rebuild a version whose digest is already pinned by a consumer.

Sign in to GHCR with read access to the old package and write access to the new one.
For the current Argo and API pin:

```sh
docker buildx imagetools create \
  --tag ghcr.io/cytiq/cytiq-engine:0.2.1 \
  ghcr.io/cytiq/cytiq-cli@sha256:66e8f4b7cf11c4a520008cf6b4a450caff7f494cf6edf4cb44e2da9ed9c024ae
docker buildx imagetools inspect \
  ghcr.io/cytiq/cytiq-engine@sha256:66e8f4b7cf11c4a520008cf6b4a450caff7f494cf6edf4cb44e2da9ed9c024ae
```

Also copy `0.3.0`, used by the local wiki, and any other versions in use. For example:

```sh
docker buildx imagetools create \
  --tag ghcr.io/cytiq/cytiq-engine:0.3.0 \
  ghcr.io/cytiq/cytiq-cli:0.3.0
```

With a single source image index, [imagetools create](https://docs.docker.com/reference/cli/docker/buildx/imagetools/create/)
copies that index unchanged. Verify the destination digest matches the source and
that both `linux/amd64` and `linux/arm64` are present. A single-platform `docker pull`
and `docker tag` is insufficient for this migration.

In **Organization → Packages → cytiq-engine → Package settings**, connect the package
to `cytiq/cytiq-engine`, grant that repository Actions write access and choose the
intended visibility. New packages default to private. Grant consuming repositories
or cluster credentials read access if it stays private. See [GHCR permissions](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry).
Keep the old package while existing deployments use it.

The CD workflow publishes future versions to the new name. The Docker check also
reads the new `cache-linux-amd64` tag; a cache miss is safe until the first release
populates it. The shared reusable workflow remains pinned to its reviewed commit.

## crates.io: first publication and trusted publishers

`cytiq-engine` is a new registry package; existing `cytiq-cli` releases retain their
name. From a clean release commit containing the rename, use the documented
[first-publication procedure](../CONTRIBUTING.md#cratesio-publication). The resumable
publisher skips versions already on crates.io and creates the new package with a
short-lived API token. Trusted publishing requires an existing crate, so complete
this before dispatching the stable release's publication workflow.

For every published workspace crate, replace the old trusted-publisher configuration
with:

| Field | Value |
| --- | --- |
| Repository owner | `cytiq` |
| Repository name | `cytiq-engine` |
| Workflow filename | `crates-publish.yml` |
| Environment | `crates-io` |

Add the same configuration for the newly published `cytiq-engine` package. crates.io
matches the repository name in the OIDC token; GitHub's URL redirect cannot satisfy
an old trusted-publisher configuration. See [crates.io trusted publishing](https://crates.io/docs/trusted-publishing).

For an existing `cargo install cytiq-cli` installation, add `--force` to the
[`cytiq-engine` install command](../crates/cytiq-engine/README.md#build-and-distribute)
to replace the `cytiq` executable owned by the old package.

## Local checkouts and verification

Update `CYTIQ_CLI_DIR` overrides in local shells or `.env` files to
`CYTIQ_ENGINE_DIR`; its default is `../cytiq-engine`. `CYTIQ_BIN` continues to name
the executable. Update any local `CLI_PACKAGE` override to `cytiq-engine`.

After renaming the checkout directory, clear old Cargo build artifacts with
`cargo clean` and rebuild. Cached test executables can retain the old absolute
`CARGO_MANIFEST_DIR` and report missing fixtures even though those files exist.

The tracked references in `cytiq-api`, `cytiq-argo`, `cytiq-ci`, `cytiq-ml`,
`cytiq-ml-automl`, `cytiq-tree` and the local wiki use the new names. Deploy them only
after checking that their images resolve in GHCR. Reapply Argo templates and rebuild
the API runner image through their normal deployment procedures.

Run `just verify` and `just docker-build` here, unit tests in `cytiq-argo`,
`cytiq-api` and `cytiq-ci`, and `argo lint --offline workflows/` in `cytiq-argo`.
The reviewed Linux CRAP baseline only has its crate identities and source paths
renamed and reordered by file/function/line; its scores and collection provenance
are preserved.
