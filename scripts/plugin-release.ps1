$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$toolArgs = $args
$pluginRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$revision = (Get-Content (Join-Path $pluginRoot 'release/toolkit-revision') -Raw).Trim()
if ($revision -cnotmatch '^[0-9a-f]{40}$') { throw 'release/toolkit-revision must be a full Git commit hash' }
$cache = Join-Path $pluginRoot "dist/toolkit/$revision"
if (-not (Test-Path (Join-Path $cache '.git'))) {
    New-Item -ItemType Directory -Force (Split-Path $cache) | Out-Null
    & git clone --quiet https://github.com/PORTALSURFER/plugin-release.git $cache
    if ($LASTEXITCODE -ne 0) { throw 'Could not clone the private toolkit; authenticate Git first.' }
    & git -C $cache checkout --quiet --detach $revision
    if ($LASTEXITCODE -ne 0) { throw 'Could not check out the pinned toolkit.' }
}
$head = & git -C $cache rev-parse HEAD
if ($LASTEXITCODE -ne 0 -or "$head".Trim() -ne $revision) { throw 'Wrong toolkit revision.' }
$dirty = & git -C $cache status --porcelain
if ($LASTEXITCODE -ne 0 -or $dirty) { throw 'Cached toolkit was modified; remove its cache and retry.' }
& cargo run --quiet --locked --manifest-path (Join-Path $cache 'Cargo.toml') --target-dir (Join-Path $cache 'target') -- --config (Join-Path $pluginRoot '.plugin-release.toml') @toolArgs
exit $LASTEXITCODE
