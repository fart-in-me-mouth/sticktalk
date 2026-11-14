[CmdletBinding()]
param([Parameter(Mandatory)][ValidatePattern('^[a-z_][a-z0-9_-]*@[a-z0-9.-]+$')][string]$Destination)
$ErrorActionPreference='Stop'
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot '../../target' }
$binary = Join-Path $target 'aarch64-unknown-linux-gnu/server/sticktalk-server'
if (!(Test-Path -LiteralPath $binary)) {throw 'Run deploy/pi/build.ps1 first.'}
& ssh $Destination 'umask 077; mkdir -p ~/sticktalk-deploy'
if ($LASTEXITCODE -ne 0) {throw 'SSH failed.'}
& scp $binary "${Destination}:sticktalk-deploy/sticktalk-server.new"
if ($LASTEXITCODE -ne 0) {throw 'Copy failed.'}
# New inode + rename: never overwrite a running ELF file. Host checks stay enabled.
& ssh $Destination 'sudo install -o root -g root -m 0755 ~/sticktalk-deploy/sticktalk-server.new /usr/local/bin/sticktalk-server.new && sudo mv /usr/local/bin/sticktalk-server.new /usr/local/bin/sticktalk-server && sudo systemctl restart sticktalk && sudo systemctl --no-pager --full status sticktalk'
if ($LASTEXITCODE -ne 0) {throw 'Deploy/restart failed. Inspect journalctl -u sticktalk.'}
