#!/usr/bin/env bash
set -u

SECTIONS="project welcome search arguments patterns filters matching walk output ranking actions explain config shell help"

usage() {
    cat <<EOF
Usage: scripts/demo.sh [options] [section...]

A guided tour of syk. Each step says what is about to happen, types the
command, runs it against a throwaway project and points out what to look at.
Your own files, shell configuration and syk data are not touched.

Keys while it runs:
  Enter   next command
  s       skip the rest of this section
  q       stop

Options:
  -a, --auto [SECONDS]   no keys needed, wait SECONDS after each command (default 3)
  -b, --by-section       wait for Enter after each section, not after each command
  -f, --fast             no waiting and no typing
  -n, --no-type          print each command at once
  -k, --keep             keep the temporary folder and print where it is
  -l, --list             list the sections
  -h, --help             show this text

Sections: $SECTIONS

The binary is taken from \$SYK, then from PATH, then from target/release.
EOF
}

mode=step
delay=3
keep=0
typing=1
chosen=""

while [ $# -gt 0 ]; do
    case "$1" in
        -a | --auto)
            mode=auto
            case "${2:-}" in
                '' | *[!0-9.]*) ;;
                *)
                    delay=$2
                    shift
                    ;;
            esac
            ;;
        -b | --by-section) mode=section ;;
        -f | --fast) mode=fast ;;
        -n | --no-type) typing=0 ;;
        -k | --keep) keep=1 ;;
        -l | --list)
            echo "$SECTIONS" | tr ' ' '\n'
            exit 0
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        -*)
            echo "demo: unknown option $1" >&2
            usage >&2
            exit 2
            ;;
        *)
            case " $SECTIONS " in
                *" $1 "*) chosen="$chosen $1" ;;
                *)
                    echo "demo: there is no section called $1. Sections: $SECTIONS" >&2
                    exit 2
                    ;;
            esac
            ;;
    esac
    shift
done
[ -n "$chosen" ] || chosen=$SECTIONS

repo=$(cd "$(dirname "$0")/.." && pwd)
if [ -n "${SYK:-}" ]; then
    binary=$SYK
elif command -v syk >/dev/null 2>&1; then
    binary=$(command -v syk)
elif [ -x "$repo/target/release/syk" ]; then
    binary=$repo/target/release/syk
else
    echo "demo: syk was not found. Install it with: cargo install --path $repo" >&2
    exit 2
fi
if [ ! -x "$binary" ]; then
    echo "demo: $binary cannot be run." >&2
    exit 2
fi
binary=$(cd "$(dirname "$binary")" && pwd)/$(basename "$binary")

if [ ! -t 0 ] && [ "$mode" != auto ]; then
    mode=fast
fi
if [ ! -t 1 ] || [ "$mode" = fast ]; then
    typing=0
fi

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    bold=$'\033[1m'
    dim=$'\033[2m'
    accent=$'\033[1;36m'
    prompt=$'\033[1;32m'
    notice=$'\033[33m'
    reset=$'\033[0m'
else
    bold=""
    dim=""
    accent=""
    prompt=""
    notice=""
    reset=""
fi

base=${TMPDIR:-/tmp}
[ -d "$base" ] || base=/tmp
sandbox=$(mktemp -d "$base/syk-demo.XXXXXX") || exit 2
if [ -z "$sandbox" ] || [ ! -d "$sandbox" ]; then
    echo "demo: could not make a temporary folder in $base." >&2
    exit 2
fi
cleanup() {
    if [ "$keep" = 1 ]; then
        printf '\nKept: %s\n' "$sandbox"
    else
        rm -rf "$sandbox"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT

export HOME="$sandbox/home"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_DATA_HOME="$HOME/.local/share"
export _ZO_DATA_DIR="$XDG_DATA_HOME/zoxide"
PATH="$sandbox/bin:$(dirname "$binary"):$PATH"
export PATH
unset SYK_CONFIG VISUAL SSH_CONNECTION
export SYK_PAGER=off
project="$HOME/shop"

days_ago() {
    date -d "$1 days ago" +%Y%m%d%H%M 2>/dev/null || date -v "-$1d" +%Y%m%d%H%M
}

build() {
    mkdir -p "$sandbox/bin" "$XDG_CONFIG_HOME/syk" "$XDG_DATA_HOME" "$HOME/shared"
    mkdir -p "$project/src/api/users" "$project/src/components" "$project/src/config" \
        "$project/src/lib" "$project/tests" "$project/dist" "$project/logs" "$project/docs"
    if [ "$(basename "$binary")" != syk ]; then
        ln -s "$binary" "$sandbox/bin/syk"
    fi

    cat >"$sandbox/bin/vim" <<'EOF'
#!/bin/sh
echo "vim was started with: $*"
EOF
    chmod +x "$sandbox/bin/vim"
    export EDITOR="$sandbox/bin/vim"

    cat >"$HOME/.bashrc" <<'EOF'
command -v zoxide >/dev/null 2>&1 && eval "$(zoxide init bash)"
EOF

    cd "$project" || exit 2
    cat >README.md <<'EOF'
# shop

A small web shop used to show what syk does.

TODO: write the install section
Run the importer with --force to overwrite existing products.
EOF
    cat >package.json <<'EOF'
{
  "name": "shop",
  "scripts": { "dev": "next dev", "test": "vitest" }
}
EOF
    printf 'dist/\n' >.gitignore
    printf 'AUTH_TOKEN=demo-token\nDATABASE_URL=postgres://localhost/shop\n' >.env
    cat >src/api/route.ts <<'EOF'
import { login } from './auth'

export async function POST(request: Request) {
  // TODO: rate limit the auth route
  const token = await login(request)
  return Response.json({ token })
}
EOF
    cat >src/api/auth.ts <<'EOF'
import { config } from '../config/config'

export function login(request: Request) {
  const user = readUser(request)
  return signToken(user, config.authSecret)
}

export function logout(token: string) {
  return revokeToken(token)
}
EOF
    cat >src/api/users/route.ts <<'EOF'
import { requireAuth } from '../auth'

export async function GET(request: Request) {
  const user = await requireAuth(request)
  const users = await listUsers(user)
  return Response.json(users)
}
EOF
    cat >src/components/Button.tsx <<'EOF'
'use client'

export function Button({ label }: { label: string }) {
  return <button className="button">{label}</button>
}
EOF
    cat >src/components/LoginForm.tsx <<'EOF'
'use client'

import { Button } from './Button'

export function LoginForm() {
  // TODO: show the auth error next to the field
  const submit = (form: FormData) => login(form)
  return <form action={submit}><Button label="Login" /></form>
}
EOF
    cat >src/config/config.ts <<'EOF'
export const config = {
  authSecret: process.env.AUTH_TOKEN,
  database: process.env.DATABASE_URL,
}
EOF
    cat >src/config/database.ts <<'EOF'
import { config } from './config'

export const database = connect(config.database)
EOF
    cat >src/lib/format.ts <<'EOF'
export function formatPrice(cents: number) {
  return (cents / 100).toFixed(2)
}

export const total = a(b) + a(c)
EOF
    cat >tests/auth.test.ts <<'EOF'
test('login returns a token', async () => {
  expect(await login(request)).toBeDefined()
})
EOF
    cat >tests/route.test.ts <<'EOF'
test('the auth route answers with a token', async () => {
  expect((await POST(request)).status).toBe(200)
})
EOF
    printf 'function login(){return auth()}\n' >dist/bundle.js
    mkdir -p node_modules/jose
    printf 'export function signToken() {}\n' >node_modules/jose/index.js
    printf 'export function signToken(){}\n' >node_modules/jose/index.min.js
    cat >docs/guide.md <<'EOF'
# Guide

Users log in through the auth route. The token is kept in a cookie.
EOF
    printf 'export function sharedAuth() {}\n' >"$HOME/shared/helpers.ts"
    ln -s ../shared shared

    yes 'GET /api/users 200 12ms' | head -c 2000000 >logs/app.log
    printf 'POST /api/auth 401 3ms\n' >logs/today.log
    touch -t "$(days_ago 40)" logs/app.log
    touch -t "$(days_ago 10)" docs/guide.md

    git init -q . 2>/dev/null || cp .gitignore .ignore
}

counting=0
skipping=0
step=0
steps=0
part=0
parts=0

ask() {
    printf '%s%s%s ' "$dim" "$1" "$reset"
    IFS= read -r -s -n 1 key
    printf '\r\033[K'
    case "$key" in
        q | Q)
            printf '\n'
            exit 0
            ;;
        s | S) skipping=1 ;;
    esac
}

typed() {
    if [ "$typing" = 0 ]; then
        printf '%s' "$1"
        return
    fi
    position=0
    while [ "$position" -lt "${#1}" ]; do
        printf '%s' "${1:$position:1}"
        sleep 0.03
        position=$((position + 1))
    done
    sleep 0.4
}

rule() {
    width=$(tput cols 2>/dev/null || echo 80)
    [ "$width" -gt 80 ] && width=80
    line=""
    while [ "${#line}" -lt $((width - ${#1} - 4)) ]; do
        line="$line-"
    done
    printf '%s-- %s %s%s\n' "$accent" "$1" "$line" "$reset"
}

heading() {
    part=$((part + 1))
    skipping=0
    [ "$counting" = 1 ] && return
    if [ "$mode" != fast ] && [ -t 1 ]; then
        printf '\033[H\033[2J'
    fi
    printf '\n'
    rule "$part/$parts  $1"
    printf '%s\n\n' "$2"
}

show() {
    [ "$counting" = 1 ] && return
    printf '%s\n\n' "$1"
    pause
}

pause() {
    case "$mode" in
        auto) sleep "$delay" ;;
        step) ask "[$step/$steps]  Enter: next   s: skip section   q: stop" ;;
    esac
}

run() {
    step=$((step + 1))
    [ "$counting" = 1 ] && return
    [ "$skipping" = 1 ] && return
    printf '%s>%s %s\n' "$accent" "$reset" "$1"
    printf '%s$%s %s' "$prompt" "$reset" "$bold"
    typed "$2"
    printf '%s\n' "$reset"
    (cd "$project" && eval "$2")
    status=$?
    if [ "$status" != 0 ]; then
        printf '%sexit status %s%s\n' "$dim" "$status" "$reset"
    fi
    if [ -n "${3:-}" ]; then
        printf '%s^ %s%s\n' "$notice" "$3" "$reset"
    fi
    printf '\n'
    pause
}

section_project() {
    heading "The project" "Every command in this tour runs in this small project, made for the occasion."
    show "shop/
  .env                          hidden, holds AUTH_TOKEN
  .gitignore                    ignores dist/
  README.md
  package.json
  dist/bundle.js                ignored
  docs/guide.md                 10 days old
  logs/app.log                  2 MB, 40 days old
  logs/today.log                new and small
  node_modules/jose/            an installed package, not in .gitignore
  shared -> ../shared           a symbolic link to a folder outside
  src/api/auth.ts               login and logout
  src/api/route.ts
  src/api/users/route.ts
  src/components/Button.tsx
  src/components/LoginForm.tsx
  src/config/config.ts
  src/config/database.ts
  src/lib/format.ts
  tests/auth.test.ts
  tests/route.test.ts"
}

section_welcome() {
    heading "First contact" "What you see when you type syk and nothing else."
    run "Without arguments syk prints a short guide." \
        "syk" \
        "Nothing was searched, so the exit status is 2."
    run "The version." \
        "syk --version"
}

section_search() {
    heading "What to search" "One word is tried on folder names, file names and the lines inside files."
    run "Search for auth everywhere at once." \
        "syk auth" \
        "A path on its own is a name that matched. Numbered lines are lines that matched."
    run "-n searches names only." \
        "syk -n route" \
        "No file was opened for this, so it is the cheapest search."
    run "-d searches folder names only." \
        "syk -d api"
    run "-c searches the lines inside files only." \
        "syk -c TODO"
    run "Two of them narrow each other: lines with auth, but only in files named *route*." \
        "syk -n route -c auth" \
        "auth.ts and LoginForm.tsx also contain auth, but their names do not match route."
    run "The same with a folder: lines with login, but only below folders named *api*." \
        "syk -d api -c login" \
        "tests/auth.test.ts has login too, but it is not below api/."
}

section_arguments() {
    heading "Arguments by shape" "syk reads what each word is from how it looks, so the order is free."
    run "A word that is a path on disk says where to search." \
        "syk auth src/api/"
    run "A word that starts with a dot is an extension." \
        "syk auth .tsx"
    run "The order does not matter." \
        "syk .tsx src/ auth" \
        "The same result as the command before."
    run "+1mb is a size and +30d is an age: log files over 1 MB that are over 30 days old." \
        "syk .log +1mb +30d" \
        "today.log is new and small, so only app.log is left."
    run "After -- nothing is read as a flag, so you can search for --force." \
        "syk -c -- --force"
}

section_patterns() {
    heading "Patterns" "Type what you are looking for. syk works out whether it is text, a wildcard or a regular expression."
    run "Lower case matches any case." \
        "syk -c login src/components/" \
        "Three lines: LoginForm, login(form) and the label Login."
    run "One upper case letter makes the case exact." \
        "syk -c Login src/components/" \
        "The line with login(form) is gone."
    run "Two words have to be on the same line, or in the same name." \
        "syk auth token" \
        "Every line shown has both auth and token in it."
    run "A regular expression works as it is." \
        "syk -c '^export (async )?function' src/api/"
    run "a(b is not a valid regular expression, so it is searched as plain text." \
        "syk -c 'a(b'"
    run "A star is a wildcard on names." \
        "syk -n '*conf*'"
    run "A typo: nothing matches confg, so a spelling one edit away is tried." \
        "syk -n confg" \
        "The first line says which spelling was used."
    run "-X turns that retry off." \
        "syk -n confg -X" \
        "Nothing found is exit status 1."
}

section_filters() {
    heading "Filters" "Narrow a search by extension, name, size, age, kind and depth."
    run "-e keeps one extension. It is the long way to write .tsx." \
        "syk -e tsx"
    run "-E leaves names out, separated by commas." \
        "syk -c auth -l -E tests,docs" \
        "Nothing from tests/ or docs/ is listed."
    run "-S +1mb keeps files of at least 1 MB. It is the long way to write +1mb." \
        "syk -S +1mb"
    run "-S -150b keeps files of at most 150 bytes." \
        "syk -c auth -l -S -150b"
    run "-M +30d keeps files last changed over 30 days ago." \
        "syk -M +30d"
    run "A bare -2h keeps what changed in the last 2 hours." \
        "syk -2h docs/ logs/" \
        "docs/guide.md is 10 days old and app.log is 40 days old, so today.log is left."
    run "-t keeps one kind of entry: file, dir or link." \
        "syk -t dir"
    run "-D limits how deep the search goes." \
        "syk -t file -D 1" \
        "Only the files directly in the project folder."
}

section_matching() {
    heading "Matching" "Override how case, special characters and word boundaries are treated."
    run "-i matches any case, even with upper case letters in the pattern." \
        "syk -c LOGIN -i tests/"
    run "-s matches the case exactly, even for a lower case pattern." \
        "syk -c login -s src/components/" \
        "Without -s this found three lines. LoginForm and Login no longer count."
    run "-F searches plain text, so the dot and the brackets mean themselves." \
        "syk -c 'toFixed(2)' -F"
    run "Without -w, user also matches users and listUsers." \
        "syk -c user src/api/users/"
    run "-w matches whole words only." \
        "syk -c user -w src/api/users/" \
        "The line with only users is gone."
}

section_walk() {
    heading "Where to look" "Hidden files, ignored files, binary files and dependency folders are skipped unless you ask for them."
    run "AUTH_TOKEN is also in .env, but hidden files are skipped." \
        "syk -c AUTH_TOKEN" \
        "Only config.ts is shown."
    run "-H includes hidden files." \
        "syk -c AUTH_TOKEN -H" \
        "Now .env is there as well."
    run "dist/ is listed in .gitignore, so it is skipped." \
        "syk -c 'function login'"
    run "-I includes what the ignore rules leave out." \
        "syk -c 'function login' -I" \
        "dist/bundle.js has joined the results."
    run "-a is -H and -I together." \
        "syk -n -a -D 1 -t file" \
        ".env and .gitignore are listed next to the visible files."
    run "node_modules is not in .gitignore, but it belongs to the group deps, which is skipped." \
        "syk -c signToken" \
        "Only auth.ts is shown."
    run "-u searches a group again." \
        "syk -c signToken -u deps" \
        "Both files of the installed package are there now."
    run "-g skips a group that is not skipped by default. min holds minified files." \
        "syk -c signToken -u deps -g min" \
        "index.min.js is gone again."
    run "shared is a symbolic link, and links are not followed." \
        "syk -c sharedAuth" \
        "Nothing found."
    run "-L follows symbolic links." \
        "syk -c sharedAuth -L"
}

section_output() {
    heading "Output" "The same results as a list, as counts, with context, or in a form other programs can read."
    run "-l prints only the paths of the files with a hit." \
        "syk -c auth -l"
    run "-N prints the number of matching lines per file." \
        "syk -c auth -N"
    run "-T adds a line of totals and the time taken." \
        "syk -c auth -N -T" \
        "The last line is the summary."
    run "-m stops after a number of results." \
        "syk -c auth -m 2"
    run "-C shows lines around each hit." \
        "syk -c TODO -C 1 src/api/" \
        "A colon marks the matching line, a dash marks the lines around it."
    run "-o prints in path order. Without it, results come as they are found." \
        "syk -n route -o"
    run "-p is the plain form that a pipe gets: one result per line, no colour." \
        "syk -c TODO -p"
    run "-P keeps colour and grouping, even into a pipe." \
        "syk -c TODO -P | head -n 4"
    run "Paths are clickable in a terminal that supports it. -K turns that off." \
        "syk -n route -K"
    run "-J prints one JSON object per result." \
        "syk -c TODO -J src/api/" \
        "spans holds where on the line the match is, counted in bytes."
    run "-0 separates paths with NUL bytes, which is what xargs -0 expects." \
        "syk .log -0 | xargs -0 wc -c"
    run "-j sets the number of worker threads." \
        "syk -c auth -l -j 2"
}

section_ranking() {
    heading "Ranking" "Results can be ordered by how well the name matches, how recent a file is and what you picked before."
    run "-r orders by relevance. The best result is printed last, next to your prompt." \
        "syk -n route -r"
    run "-W shows the score of each result and what it is made of." \
        "syk -n route -W" \
        "The first line is the formula. Each column is one part of the score."
    run "-R prints results as they are found, without ranking." \
        "syk -n route -R"
    run "-1 prints only the best match and remembers that you chose it." \
        "syk -d components -1"
    run "Chosen a second time." \
        "syk -d components -1"
    run "-k lists what has been remembered." \
        "syk -k" \
        "Two picks for the components folder."
    run "A remembered path scores higher from then on." \
        "syk -d co -W" \
        "components gets 4 points in the picked column, config gets none."
    if [ "$mode" = step ] || [ "$mode" = section ]; then
        run "-b shows the results in a list. Move with j and k, press Enter to choose one." \
            "syk -n route -b" \
            "The chosen path is printed, and remembered like a pick made with -1."
    fi
}

section_actions() {
    heading "Actions" "Do something with the results: open the best one, or run a command on all of them."
    run "-O opens the best hit in your editor, at the matching line. A stand-in plays vim here." \
        "syk -c 'function logout' -O" \
        "The editor got +8: line 8 of auth.ts."
    run "-f runs a command with every result added at the end." \
        "syk .ts src/api/ -f 'wc -l'" \
        "wc ran once, with three files."
    run "With {} in it, the command runs once per result." \
        "syk .log -f 'echo found {}'"
    run "Put together: compress every log that is over 30 days old." \
        "syk .log +30d -f 'gzip -k {}' && ls logs/" \
        "app.log.gz is new. today.log was too recent."
}

section_explain() {
    heading "Explain" "-x shows how your arguments are read, without searching."
    run "Each word gets a role: pattern, place, filter." \
        "syk auth src/ .ts +100b -x"
    run "With two targets it shows how they narrow each other." \
        "syk -n route -c auth -E tests -x"
}

section_config() {
    heading "Config file" "Flags you always want go in a file, written the way you would type them."
    run "Write a config file: always include hidden files, always leave out tests." \
        "printf -- '-H\n-E tests\n' > ~/.config/syk/config"
    run "From now on those flags are part of every search." \
        "syk -c token" \
        ".env is included and nothing from tests/ is shown."
    run "-x shows which defaults were added and from which file." \
        "syk -c token -x" \
        "See the defaults row at the bottom."
    run "-U ignores the config file for one command." \
        "syk -c token -U" \
        ".env is gone and tests/ is back."
    run "Remove it again." \
        "rm ~/.config/syk/config"
}

section_shell() {
    heading "Shell setup" "Tab completion and a jump command. This runs in a throwaway home folder, not in yours."
    run "--init prints the shell script. This is the start of the one for fish." \
        "syk --init fish | head -n 12" \
        "s is the jump command: it changes to the best matching folder."
    run "The shell configuration before setup. zoxide is loaded there." \
        "cat ~/.bashrc"
    run "--setup looks at how zoxide is loaded and proposes the same for syk. Answered with y here." \
        "printf 'y\n' | syk --setup bash" \
        "It asked first, wrote one line, then started a new bash to check that s exists."
    run "The shell configuration after setup." \
        "cat ~/.bashrc"
    run "The jump command in a new shell: s with part of a folder name." \
        "bash -ic 's comp && pwd' 2>/dev/null" \
        "The shell is now in src/components."
    run "A second run finds the line and changes nothing." \
        "syk --setup bash"
    run "For configurations it cannot work out, it prints instructions for a coding agent." \
        "syk --setup agent | head -n 9"
}

section_help() {
    heading "Help" "Every flag has a short form, a long form and a page of its own."
    run "The start of the full help. In a pipe the logo is left out." \
        "syk -h | head -n 22"
    run "Put -h next to any flag for its page." \
        "syk --size --help"
    run "Or name the flag after -h." \
        "syk -h exec"
    run "Help taller than the window opens in a pager: n next, b back, / search, q quit." \
        "syk -h -h"
    run "A word that is not the name of a flag is looked up in every flag page." \
        "syk -h zoxide"
    run "-B leaves the logo out and -G prints a view in one piece." \
        "syk -n -h -B -G"
    run "A wrong flag is exit status 2, with a suggestion." \
        "syk auth --nope"
}

build

counting=1
for name in $chosen; do
    "section_$name"
done
counting=0
steps=$step
parts=$part
step=0
part=0

printf '%ssyk tour%s  %s%s, %s%s\n' "$bold" "$reset" "$dim" "$("$binary" --version)" "$binary" "$reset"
if [ "$parts" = 1 ]; then
    printf '%s%s commands in 1 section.%s\n' "$dim" "$steps" "$reset"
else
    printf '%s%s commands in %s sections.%s\n' "$dim" "$steps" "$parts" "$reset"
fi
case "$mode" in
    step | section) ask "Enter: start   q: stop" ;;
esac

for name in $chosen; do
    if [ "$mode" = section ] && [ "$part" != 0 ]; then
        ask "[$part/$parts]  Enter: next section   q: stop"
    fi
    "section_$name"
done
printf '%sThat was the tour. syk -h lists every flag.%s\n' "$dim" "$reset"
