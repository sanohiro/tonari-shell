# These Bash archives match the GNU tests used by the distribution audits.
download_bash() {
  local version=$1 hash=$2 path=$3
  curl --retry 3 -fsSL "https://ftpmirror.gnu.org/bash/bash-$version.tar.gz" -o "$path" ||
    curl --retry 3 -fsSL "https://ftp.gnu.org/gnu/bash/bash-$version.tar.gz" -o "$path"
  echo "$hash  $path" | shasum -a 256 -c -
}
version=5.2.37
digest=9599b22ecd1d5787ad7d3b7bf0c59f312b3396d1e281175dd1f8a4014da621ff
