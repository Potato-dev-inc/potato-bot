#!/bin/sh
# The person's file browser for the bot's computer: list a folder, look at a file, read a file, put a file in.
# Written for Puppet Master (no Rakazo code). The app runs this text with `sh -c SCRIPT sh OP ARG...` as the sandbox user,
# through the app's one door into the computer; nothing is installed in the guest and the bot cannot call it.
#
#   list  PATH                    the folder's entries, NUL-separated records (first record: P<TAB>path from home)
#   stat  PATH                    one line: d|f|o<TAB>size
#   head  PATH MAX                the first MAX bytes of a regular file
#   get   PATH MAX                the whole file, or exit 4 when it is bigger than MAX bytes
#   put   DIR NAME MAX            stdin becomes DIR/NAME (at most MAX bytes); an existing name is never replaced
#
# Every PATH is checked here, in the computer, where the symbolic links are real:
#   - empty, or with a `..` part: refused (10, 11);
#   - an absolute path must start at the home folder, before the disk is touched (5);
#   - the path is then resolved through every link (`realpath -m`), and the RESULT must be the home folder or inside it (5);
#   - a result whose first part starts with a dot (.pm-sbx with the screen passwords, .browser-profiles with the bot's
#     logins, .ssh, .config ...) is refused, so a link inside home cannot lead there either (5);
#   - the folders that hold the tool secrets, the boot state and the hub logs (/opt, /run, /var/lib) are outside home,
#     so they are refused by the same rule. A path outside home looks the same whether or not it exists.
# Exit codes: 0 ok, 2 not found, 3 not a folder, 4 too big, 5 outside or hidden, 6 name taken, 7 not a regular file,
# 8 could not write, 10 empty path, 11 has .., 12 bad name, 20 no home folder.
# POSIX sh only (the guest's /bin/sh is dash). PM_FILES_HOME is for the tests; in the computer it is /home/pm.

set -u
HOME_ROOT=${PM_FILES_HOME:-/home/pm}
HOME_REAL=$(realpath -e -- "$HOME_ROOT" 2>/dev/null) || exit 20

# resolve ARG -> R (real path inside home) and REL (the part after home; empty for home itself).
resolve() {
  [ -n "$1" ] || exit 10
  case "/$1/" in */../*) exit 11 ;; esac
  case $1 in
    "~") c=$HOME_ROOT ;;
    "~/"*) c=$HOME_ROOT/${1#"~/"} ;;
    /*) c=$1 ;;
    *) c=$HOME_ROOT/$1 ;;
  esac
  case $c in "$HOME_ROOT" | "$HOME_ROOT"/*) ;; *) exit 5 ;; esac
  R=$(realpath -m -- "$c" 2>/dev/null) || exit 5
  case $R in
    "$HOME_REAL") REL="" ;;
    "$HOME_REAL"/*) REL=${R#"$HOME_REAL"/} ;;
    *) exit 5 ;;
  esac
  case $REL in .*) exit 5 ;; esac
}

OP=${1:-}
[ $# -gt 0 ] && shift

case $OP in
  list)
    resolve "${1:-"~"}"
    [ -e "$R" ] || exit 2
    [ -d "$R" ] || exit 3
    printf 'P\t%s\0' "$REL"
    if [ -z "$REL" ]; then
      find "$R" -mindepth 1 -maxdepth 1 ! -name '.*' -printf '%y\t%Y\t%s\t%T@\t%f\0' | head -z -n 2001
    else
      find "$R" -mindepth 1 -maxdepth 1 -printf '%y\t%Y\t%s\t%T@\t%f\0' | head -z -n 2001
    fi
    ;;
  stat)
    resolve "${1:-}"
    [ -e "$R" ] || exit 2
    if [ -d "$R" ]; then t=d; elif [ -f "$R" ]; then t=f; else t=o; fi
    s=$(stat -c %s -- "$R" 2>/dev/null) || exit 2
    printf '%s\t%s\n' "$t" "$s"
    ;;
  head | get)
    resolve "${1:-}"
    max=${2:-0}
    case $max in "" | *[!0-9]*) exit 12 ;; esac
    [ -e "$R" ] || exit 2
    [ -f "$R" ] || exit 7
    if [ "$OP" = get ]; then
      s=$(stat -c %s -- "$R" 2>/dev/null) || exit 2
      [ "$s" -le "$max" ] || exit 4
    fi
    head -c "$max" -- "$R"
    ;;
  put)
    resolve "${1:-}"
    name=${2:-}
    max=${3:-0}
    case $max in "" | *[!0-9]*) exit 12 ;; esac
    [ -e "$R" ] || exit 2
    [ -d "$R" ] || exit 3
    case $name in "" | . | .. | */*) exit 12 ;; esac
    if [ -z "$REL" ]; then case $name in .*) exit 5 ;; esac; fi
    dest=$R/$name
    if [ -e "$dest" ] || [ -L "$dest" ]; then exit 6; fi
    tmp=$R/.pm-upload.$$
    head -c "$((max + 1))" >"$tmp" || { rm -f -- "$tmp"; exit 8; }
    s=$(stat -c %s -- "$tmp" 2>/dev/null) || { rm -f -- "$tmp"; exit 8; }
    if [ "$s" -gt "$max" ]; then rm -f -- "$tmp"; exit 4; fi
    # A hard link fails when the name exists (even as a link), so a file that appeared meanwhile is never replaced.
    if ln -- "$tmp" "$dest" 2>/dev/null; then rm -f -- "$tmp"; else rm -f -- "$tmp"; exit 6; fi
    if [ -z "$REL" ]; then printf '%s\n' "$name"; else printf '%s/%s\n' "$REL" "$name"; fi
    ;;
  *)
    exit 12
    ;;
esac
