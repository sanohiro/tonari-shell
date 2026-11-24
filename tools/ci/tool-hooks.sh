#!/usr/bin/env bash
# Native Linux fixtures. All installations are scoped to RUNNER_TEMP.
set -euo pipefail
bin="$RUNNER_TEMP/bin"
assets="$RUNNER_TEMP/tools"
mkdir -p "$assets"
cd "$assets"
fetch() {
  curl --retry 3 -fsSL "$1" -o "$2"
  echo "$3  $2" | sha256sum --check --status
}
case $(uname -m) in
  x86_64)
    arch=x64
    goarch=amd64
    direnv_hash=148820a187930d36ca6a204504e5cdb30d8396cd1a97e64c3b3f10d793c11247
    mise_hash=6fa596291e9758106f761fea4f8e7fac6c3e7ff4714d5c0b9a76e16068db14f6
    fzf_hash=cf919f05b7581b4c744d764eaa704665d61dd6d3ca785f0df2351281dff60cda
    n24=76b77c43f97a7c4cf60917e24c5e212faaf8ca664e433759ef98d12f268b6b49
    n18=7f5d6922a91986ef059ba8a4396aa435440adacfe6fc6fab60a857c8f2cf5e7a ;;
  aarch64)
    arch=arm64
    goarch=arm64
    direnv_hash=ec2a32dbe2f19dd085bd9315a1b4ea88b55ec288aa7eabe8723490baf466cb23
    mise_hash=7bd3b2d5ffff077a7662ba186c09600e5b40c1eee9df0c264ed67642ce0a8698
    fzf_hash=bd9e6165ebdb702215d42368cbb95b8dd70a4e77ee97925adac8c31660e30ef7
    n24=ff5a3a68781794a473f59a727f35314723785eff6f48aaf983361fadf0eadf36
    n18=5b338667822341d1ea3b18d5b37d442a655829b9eafdc5f9008f00b8451ac148 ;;
  *) exit 2 ;;
esac
github=https://github.com
raw=https://raw.githubusercontent.com
fetch "$github/direnv/direnv/releases/download/v2.38.2/direnv.linux-$goarch" direnv "$direnv_hash"
fetch "$github/jdx/mise/releases/download/v2026.11.14/mise-v2026.11.14-linux-$arch" mise "$mise_hash"
fetch "$github/junegunn/fzf/releases/download/v0.74.0/fzf-0.74.0-linux_$goarch.tar.gz" fzf.tar.gz "$fzf_hash"
tar -xzf fzf.tar.gz fzf
chmod +x direnv mise fzf
fetch "$raw/nvm-sh/nvm/v0.40.3/nvm.sh" nvm.sh 390260ab9eb1da20e8bc0ebea2ee90f528d53e5e9f6e13b16717db4af454df9d
fetch "$raw/git/git/v2.54.0/contrib/completion/git-completion.bash" git.bash 56264a168d862c2a51305fe2a3450edf04590d8bafa42c2d061e6f992985fc0f
fetch "$raw/microsoft/vscode/2c33b50b16c472efe58357d1d2a8a5414bc23fbf/src/vs/workbench/contrib/terminal/common/scripts/shellIntegration-bash.sh" vscode.sh d332cf4f442951caf33f3280a9f33cdfc50c80cdfa644edf73db48e02c2b0734
for v in 24.1.0 18.13.0; do
  hash=$n18
  [[ $v == 24.1.0 ]] && hash=$n24
  fetch "https://nodejs.org/dist/v$v/node-v$v-linux-$arch.tar.xz" node.tar.xz "$hash"
  mkdir "node-$v"
  tar -xf node.tar.xz -C "node-$v" --strip-components=1
done
cd "$GITHUB_WORKSPACE"
export PATH="$assets:/usr/local/bin:/usr/bin:/bin"
git --version
gh --version
if [[ ! -r /usr/share/bash-completion/bash_completion ]]; then sudo apt-get install -y bash-completion; fi
options=(--runs 15 --git-completion "$assets/git.bash" --bash-completion /usr/share/bash-completion/bash_completion
         --nvm-script "$assets/nvm.sh" --nodes "$assets/node-24.1.0" "$assets/node-18.13.0" --vscode-script "$assets/vscode.sh")
python3 tools/bench_tool_hooks.py "$bin/current" "$bin/baseline" /bin/bash \
  --case git --case gh --case nvm --case mise --case direnv --case vscode "${options[@]}"
python3 tools/bench_tool_hooks.py "$bin/current" "$bin/baseline" /bin/bash --case combined "${options[@]}"
python3 tools/bench_tool_hooks.py "$bin/current" "$bin/baseline" /bin/bash --case fzf-files --runs 15
for mode in live saved; do
  flags=()
  if [[ $mode == saved ]]; then flags+=(--saved-history); fi
  python3 tools/bench_tool_hooks.py "$bin/current" /bin/bash --case fzf --runs 15 "${flags[@]}"
done
