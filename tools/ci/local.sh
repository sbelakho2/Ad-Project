#!/bin/sh
# local.sh: run the Woodpecker lanes locally in the same pinned images.
# Lanes, steps, commands, services and matrix entries are parsed from
# .woodpecker/*.yml, never hardcoded. Full notes live in
# docs/Internal/CI/LOCAL_CI.md. DOGFOOD_DRY_RUN=1 equals --dry-run.
set -eu

WS=/woodpecker/src/local

usage() {
  cat <<'EOF'
usage: sh tools/ci/local.sh [flags]

Run the Woodpecker lanes in .woodpecker/*.yml locally, in the same images,
with the same commands, mounted at the CI_WORKSPACE path the lanes use.

flags:
  --lane <name>      run one lane (repeatable; default: all lanes)
  --list             list lanes, steps, services and images; run nothing
  --keep             keep containers, network and step scripts for debugging
  --dry-run          print the docker commands; execute nothing

environment:
  APEX_CI_PLATFORM   force the docker platform (for example linux/amd64);
                     default: linux/arm64 on Apple Silicon, linux/amd64 on
                     x86-64
  APEX_CI_LANES_DIR  lane directory override (default: .woodpecker)
  DOGFOOD_DRY_RUN=1  same as --dry-run
EOF
}

die() {
  printf '%s\n' "local-ci: $*" >&2
  exit 1
}

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)

SELECTED=
LIST=0
KEEP=0
DRY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --lane)
      [ $# -ge 2 ] || die "--lane needs a lane name"
      SELECTED="$SELECTED $2"
      shift 2
      ;;
    --list)
      LIST=1
      shift
      ;;
    --keep)
      KEEP=1
      shift
      ;;
    --dry-run)
      DRY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown flag: '$1' (see --help)"
      ;;
  esac
done

if [ "${DOGFOOD_DRY_RUN:-0}" = 1 ]; then
  DRY=1
fi

LANES_DIR="${APEX_CI_LANES_DIR:-$ROOT/.woodpecker}"
[ -d "$LANES_DIR" ] || die "lane directory not found: $LANES_DIR"
ls "$LANES_DIR"/*.yml >/dev/null 2>&1 || die "no lane files (*.yml) in $LANES_DIR"

PLATFORM="${APEX_CI_PLATFORM:-}"
if [ -z "$PLATFORM" ]; then
  HOST_ARCH=$(uname -m)
  case "$HOST_ARCH" in
    x86_64|amd64)
      PLATFORM=linux/amd64
      ;;
    arm64|aarch64)
      PLATFORM=linux/arm64
      ;;
    *)
      die "unsupported host architecture '$HOST_ARCH'; set APEX_CI_PLATFORM (for example linux/amd64)"
      ;;
  esac
fi
DOCKER_DEFAULT_PLATFORM="$PLATFORM"
export DOCKER_DEFAULT_PLATFORM

command -v python3 >/dev/null 2>&1 || die "python3 is required to parse the lane files"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/apex-ci.XXXXXX")
TAB=$(printf '\t')
STREAM="$TMP/plan.tsv"
SCRIPTS_DIR="$TMP/steps"
mkdir -p "$SCRIPTS_DIR"

NET=""
SVC_CONTAINERS=""

cleanup() {
  if [ "$KEEP" = 1 ]; then
    return 0
  fi
  if [ -n "$NET" ] && [ "$DRY" = 0 ]; then
    for c in $SVC_CONTAINERS; do
      docker rm -f "$c" >/dev/null 2>&1 || :
    done
    docker network rm "$NET" >/dev/null 2>&1 || :
  fi
  rm -rf "$TMP"
}

trap cleanup EXIT
trap 'exit 130' INT HUP TERM

if ! python3 - "$LANES_DIR" > "$STREAM" <<'PYEOF'
import os
import sys


def fail(msg):
    sys.stderr.write("lane-parse: %s\n" % msg)
    sys.exit(2)


def strip_comment(s):
    out = []
    quote = None
    i = 0
    while i < len(s):
        ch = s[i]
        if quote is not None:
            out.append(ch)
            if quote == '"' and ch == '\\' and i + 1 < len(s):
                out.append(s[i + 1])
                i += 2
                continue
            if ch == quote:
                quote = None
            i += 1
            continue
        if ch == "'" or ch == '"':
            quote = ch
            out.append(ch)
        elif ch == '#' and (i == 0 or s[i - 1] in ' \t'):
            break
        else:
            out.append(ch)
        i += 1
    return ''.join(out).rstrip()


def unquote(s):
    s = s.strip()
    if len(s) >= 2 and s[0] == s[-1] and s[0] in "'\"":
        return s[1:-1]
    return s


def split_flow(s):
    items = []
    cur = []
    quote = None
    for ch in s:
        if quote is not None:
            cur.append(ch)
            if ch == quote:
                quote = None
            continue
        if ch == "'" or ch == '"':
            quote = ch
            cur.append(ch)
        elif ch == ',':
            items.append(unquote(''.join(cur)))
            cur = []
        else:
            cur.append(ch)
    items.append(unquote(''.join(cur)))
    return [x for x in items if x != '']


def parse_scalar(v):
    v = v.strip()
    if v.startswith('[') and v.endswith(']'):
        return split_flow(v[1:-1])
    return unquote(v)


def split_kv(content):
    quote = None
    for i, ch in enumerate(content):
        if quote is not None:
            if ch == quote:
                quote = None
            continue
        if ch == "'" or ch == '"':
            quote = ch
            continue
        if ch == ':' and (i + 1 == len(content) or content[i + 1] == ' '):
            return content[:i].strip(), content[i + 1:].strip()
    return None


def preprocess(text, path):
    lines = []
    for no, raw in enumerate(text.splitlines(), 1):
        lead = raw[:len(raw) - len(raw.lstrip(' \t'))]
        if '\t' in lead:
            fail("%s:%d: tab indentation is not supported" % (path, no))
        stripped = strip_comment(raw)
        if not stripped.strip():
            continue
        indent = len(stripped) - len(stripped.lstrip(' '))
        lines.append([indent, stripped.strip(), no])
    return lines


def child_indent(lines, i, indent):
    if i < len(lines) and lines[i][0] > indent:
        return lines[i][0]
    return None


def parse_block(lines, i, indent, path):
    if i >= len(lines) or lines[i][0] < indent:
        return None, i
    if lines[i][1].startswith('-'):
        seq = []
        while i < len(lines) and lines[i][0] == indent and lines[i][1].startswith('-'):
            raw = lines[i][1]
            content = raw[1:].strip()
            if content == '':
                nested = child_indent(lines, i + 1, indent)
                if nested is None:
                    seq.append(None)
                    i += 1
                else:
                    value, i = parse_block(lines, i + 1, nested, path)
                    seq.append(value)
                continue
            if split_kv(content) is not None:
                key_indent = indent + raw.index(content)
                lines[i] = [key_indent, content, lines[i][2]]
                value, i = parse_block(lines, i, key_indent, path)
                seq.append(value)
                continue
            seq.append(parse_scalar(content))
            i += 1
        return seq, i
    mapping = {}
    while i < len(lines) and lines[i][0] == indent and not lines[i][1].startswith('-'):
        content = lines[i][1]
        kv = split_kv(content)
        if kv is None:
            fail("%s:%d: cannot parse line: %s" % (path, lines[i][2], content))
        key, rest = kv
        if rest == '':
            nested = child_indent(lines, i + 1, indent)
            if nested is None:
                value = None
                i += 1
            else:
                value, i = parse_block(lines, i + 1, nested, path)
        elif rest == '|' or rest == '>':
            fail("%s:%d: block scalars are not supported by this parser" % (path, lines[i][2]))
        else:
            value = parse_scalar(rest)
            i += 1
        mapping[key] = value
    return mapping, i


def parse_document(text, path):
    lines = preprocess(text, path)
    if not lines:
        fail("%s: empty file" % path)
    doc, i = parse_block(lines, 0, lines[0][0], path)
    if i != len(lines):
        fail("%s:%d: could not parse the whole file" % (path, lines[i][2]))
    return doc


def as_list(value, what, path):
    if value is None:
        return []
    if isinstance(value, list):
        return value
    if isinstance(value, str):
        return [value]
    fail("%s: %s must be a list of strings" % (path, what))


def load_lane(path):
    name = os.path.basename(path)
    with open(path, encoding='utf-8') as fh:
        doc = parse_document(fh.read(), name)
    if not isinstance(doc, dict):
        fail("%s: top level must be a mapping" % name)
    lane = {
        'name': name[:-4],
        'services': [],
        'steps': [],
        'depends_on': as_list(doc.get('depends_on'), 'depends_on', name),
        'matrix': [],
    }
    services = doc.get('services') or {}
    if not isinstance(services, dict):
        fail("%s: services must be a mapping" % name)
    for svc, sdef in services.items():
        if not isinstance(sdef, dict) or not isinstance(sdef.get('image'), str):
            fail("%s: service '%s' has no image" % (name, svc))
        env = sdef.get('environment') or {}
        if not isinstance(env, dict):
            fail("%s: service '%s' environment must be a mapping" % (name, svc))
        lane['services'].append({
            'name': svc,
            'image': sdef['image'],
            'env': [(str(k), str(v)) for k, v in env.items()],
        })
    matrix = doc.get('matrix') or {}
    if isinstance(matrix, dict) and matrix.get('include'):
        include = matrix['include']
        if not isinstance(include, list):
            fail("%s: matrix.include must be a list" % name)
        for entry in include:
            if not isinstance(entry, dict):
                fail("%s: matrix entries must be mappings" % name)
            lane['matrix'].append([(str(k), str(v)) for k, v in entry.items()])
    steps = doc.get('steps') or {}
    if not isinstance(steps, dict) or not steps:
        fail("%s: no steps" % name)
    for step, sdef in steps.items():
        if not isinstance(sdef, dict):
            fail("%s: step '%s' must be a mapping" % (name, step))
        image = sdef.get('image')
        if not isinstance(image, str) or not image:
            fail("%s: step '%s' has no image" % (name, step))
        cmds = sdef.get('commands')
        if not isinstance(cmds, list) or not cmds:
            fail("%s: step '%s' has no commands" % (name, step))
        clean = []
        for c in cmds:
            if not isinstance(c, str) or not c.strip():
                fail("%s: step '%s' has an empty command" % (name, step))
            clean.append(c.strip())
        lane['steps'].append({
            'name': step,
            'image': image,
            'commands': clean,
            'depends_on': as_list(sdef.get('depends_on'), 'depends_on', name),
        })
    return lane


def topo(names, deps_of):
    remaining = list(names)
    order = []
    done = set()
    while remaining:
        progressed = False
        rest = []
        for n in remaining:
            if all((d in done) or (d not in names) for d in deps_of(n)):
                order.append(n)
                done.add(n)
                progressed = True
            else:
                rest.append(n)
        if not progressed:
            fail("dependency cycle among: %s" % ", ".join(rest))
        remaining = rest
    return order


def emit(*fields):
    sys.stdout.write("\t".join(fields) + "\n")


def main():
    dirpath = sys.argv[1]
    files = sorted(f for f in os.listdir(dirpath) if f.endswith('.yml'))
    if not files:
        fail("no *.yml lane files in %s" % dirpath)
    lanes = [load_lane(os.path.join(dirpath, f)) for f in files]
    lane_names = [l['name'] for l in lanes]
    lane_deps = {}
    for l in lanes:
        lane_deps[l['name']] = [d for d in l['depends_on'] if d in lane_names]
    for l in lanes:
        step_names = [s['name'] for s in l['steps']]
        step_deps = {}
        for s in l['steps']:
            step_deps[s['name']] = [d for d in s['depends_on'] if d in step_names]
            ignored = [d for d in s['depends_on'] if d not in step_names]
            if ignored:
                sys.stderr.write(
                    "lane-parse: note: lane %s step '%s' depends on %s, which is not a "
                    "step in that lane; it is a lane or workflow dependency and is not "
                    "ordered locally\n" % (l['name'], s['name'], ", ".join(ignored)))
        l['run_order'] = topo(step_names, lambda n: step_deps[n])
        l['step_deps'] = step_deps
        l['steps_by_name'] = {s['name']: s for s in l['steps']}
    lanes_by_name = {}
    for l in lanes:
        lanes_by_name[l['name']] = l
    for lane_name in topo(lane_names, lambda n: lane_deps[n]):
        lane = lanes_by_name[lane_name]
        deps = lane_deps[lane_name]
        emit('LANE', lane['name'], ",".join(deps) if deps else '-', '-', '-')
        for svc in lane['services']:
            env = ",".join("%s=%s" % (k, v) for k, v in svc['env'])
            emit('SERVICE', lane['name'], svc['name'], svc['image'], '-', env if env else '-')
        for step in lane['run_order']:
            sdef = lane['steps_by_name'][step]
            entries = lane['matrix'] if lane['matrix'] else [[]]
            for entry in entries:
                env = ",".join("%s=%s" % (k, v) for k, v in entry)
                label = step
                if env:
                    label = "%s[%s]" % (step, env)
                emit('STEP', lane['name'], label, sdef['image'],
                     ",".join(lane['step_deps'][step]) if lane['step_deps'][step] else '-',
                     env if env else '-')
                for cmd in sdef['commands']:
                    emit('CMD', lane['name'], label, '-', '-', cmd.replace("\t", "    "))


main()
PYEOF
then
  die "lane parsing failed (see above)"
fi

if [ "$LIST" = 1 ]; then
  while IFS="$TAB" read -r kind f1 f2 f3 f4 f5; do
    case "$kind" in
      LANE)
        if [ "$f2" = "-" ]; then
          printf 'lane %s\n' "$f1"
        else
          printf 'lane %s (depends_on: %s)\n' "$f1" "$f2"
        fi
        ;;
      SERVICE)
        if [ "$f5" = "-" ]; then
          printf '  service %s image=%s\n' "$f2" "$f3"
        else
          printf '  service %s image=%s env=%s\n' "$f2" "$f3" "$f5"
        fi
        ;;
      STEP)
        if [ "$f5" = "-" ]; then
          if [ "$f4" = "-" ]; then
            printf '  step %s image=%s\n' "$f2" "$f3"
          else
            printf '  step %s image=%s depends_on=%s\n' "$f2" "$f3" "$f4"
          fi
        else
          printf '  step %s image=%s env=%s depends_on=%s\n' "$f2" "$f3" "$f5" "$f4"
        fi
        ;;
    esac
  done < "$STREAM"
  exit 0
fi

ALL_LANES=$(awk -F"$TAB" '$1 == "LANE" { printf "%s ", $2 }' "$STREAM")
for sel in $SELECTED; do
  case " $ALL_LANES " in
    *" $sel "*) : ;;
    *) die "unknown lane '$sel'; known lanes: $ALL_LANES" ;;
  esac
done

if [ "$DRY" = 1 ]; then
  :
else
  command -v docker >/dev/null 2>&1 || die "docker is not available; install Docker or use --list/--dry-run"
  if ! docker info >/dev/null 2>&1; then
    die "the docker daemon is not reachable; start Docker or use --list/--dry-run"
  fi
fi

is_selected() {
  if [ -z "$SELECTED" ]; then
    return 0
  fi
  case " $SELECTED " in
    *" $1 "*) return 0 ;;
  esac
  return 1
}

quote_arg() {
  case "$1" in
    '')
      printf "''"
      ;;
    *[!A-Za-z0-9_/:=\.,@%+-]*)
      printf "'"
      printf '%s' "$1" | sed "s/'/'\\\\''/g"
      printf "'"
      ;;
    *)
      printf '%s' "$1"
      ;;
  esac
}

print_cmd() {
  line=""
  for a in "$@"; do
    line="$line $(quote_arg "$a")"
  done
  printf '%s\n' "${line# }"
}

CUR_LANE=""
LANE_SELECTED=0
LANE_HAS_SVC=0
CUR_LABEL=""
CUR_IMAGE=""
CUR_ENV="-"
CUR_SCRIPT=""
STEP_NO=0
STATUS_FILE="$TMP/status"
LOG_FILE="$TMP/last-step.log"

finish_step() {
  if [ -z "$CUR_LABEL" ]; then
    return 0
  fi
  STEP_NO=$((STEP_NO + 1))
  slug=$(printf '%s' "$CUR_LANE-$CUR_LABEL" | tr -c 'a-zA-Z0-9._-' '-')
  name="apex-ci-$slug-$STEP_NO"
  set -- docker run
  if [ "$KEEP" = 0 ]; then
    set -- "$@" --rm
  fi
  set -- "$@" -i --name "$name" --platform "$PLATFORM" -e "CI_WORKSPACE=$WS" -e "CI=true" -v "$ROOT:$WS" -w "$WS"
  if [ "$LANE_HAS_SVC" = 1 ]; then
    set -- "$@" --network "$NET"
  fi
  if [ "$CUR_ENV" != "-" ]; then
    for kv in $(printf '%s' "$CUR_ENV" | tr ',' ' '); do
      set -- "$@" -e "$kv"
    done
  fi
  set -- "$@" "$CUR_IMAGE" sh -es
  if [ "$DRY" = 1 ]; then
    print_cmd "$@"
    printf '%s\n' "<<'APEX_CI_SCRIPT'"
    cat "$CUR_SCRIPT"
    printf '%s\n' "APEX_CI_SCRIPT"
    CUR_LABEL=""
    return 0
  fi
  printf '%s\n' "local-ci: step $CUR_LANE/$CUR_LABEL ($CUR_IMAGE)"
  : > "$STATUS_FILE"
  ( set +e; "$@" < "$CUR_SCRIPT" 2>&1; printf '%s' "$?" > "$STATUS_FILE" ) | tee "$LOG_FILE"
  rc=$(cat "$STATUS_FILE")
  if [ "$rc" -ne 0 ]; then
    printf '%s\n' "local-ci: FAILED lane '$CUR_LANE' step '$CUR_LABEL' (exit $rc)" >&2
    printf '%s\n' "local-ci: tail of the step output:" >&2
    tail -n 40 "$LOG_FILE" >&2
    if [ "$KEEP" = 1 ]; then
      printf '%s\n' "local-ci: --keep: containers, network and scripts survive for inspection" >&2
    fi
    exit "$rc"
  fi
  if [ "$KEEP" = 1 ]; then
    printf '%s\n' "local-ci: --keep: step container $name left in place"
  fi
  CUR_LABEL=""
}

finish_lane() {
  finish_step
  if [ -n "$NET" ] && [ "$KEEP" = 0 ] && [ "$DRY" = 0 ]; then
    for c in $SVC_CONTAINERS; do
      docker rm -f "$c" >/dev/null 2>&1 || :
    done
    docker network rm "$NET" >/dev/null 2>&1 || :
    SVC_CONTAINERS=""
    NET=""
  fi
  LANE_HAS_SVC=0
}

while IFS="$TAB" read -r kind f1 f2 f3 f4 f5; do
  case "$kind" in
    LANE)
      finish_lane
      CUR_LANE="$f1"
      if [ -n "$NET" ] && [ "$DRY" = 0 ]; then
        printf '%s\n' "local-ci: --keep: network $NET and service containers stay up"
      fi
      NET=""
      SVC_CONTAINERS=""
      if is_selected "$f1"; then
        LANE_SELECTED=1
        if [ "$DRY" = 1 ]; then
          printf '# lane %s (platform %s)\n' "$f1" "$PLATFORM"
        else
          printf '%s\n' "local-ci: lane $f1 (platform $PLATFORM)"
        fi
      else
        LANE_SELECTED=0
      fi
      ;;
    SERVICE)
      [ "$LANE_SELECTED" = 1 ] || continue
      if [ -z "$NET" ]; then
        NET="apex-ci-$f1"
        if [ "$DRY" = 1 ]; then
          print_cmd docker network create "$NET"
        else
          docker network create "$NET" >/dev/null
        fi
      fi
      LANE_HAS_SVC=1
      svc_name="apex-ci-$f1-$f2"
      set -- docker run -d --name "$svc_name" --network "$NET" --network-alias "$f2" --platform "$PLATFORM"
      if [ "$f5" != "-" ]; then
        for kv in $(printf '%s' "$f5" | tr ',' ' '); do
          set -- "$@" --env "$kv"
        done
      fi
      set -- "$@" "$f3"
      if [ "$DRY" = 1 ]; then
        print_cmd "$@"
      else
        if ! "$@" >/dev/null; then
          die "service $f2 ($f3) failed to start"
        fi
        printf '%s\n' "local-ci: service $f2 ($f3) up as $svc_name on $NET"
      fi
      SVC_CONTAINERS="$SVC_CONTAINERS $svc_name"
      ;;
    STEP)
      [ "$LANE_SELECTED" = 1 ] || continue
      finish_step
      CUR_LABEL="$f2"
      CUR_IMAGE="$f3"
      CUR_ENV="$f5"
      CUR_SCRIPT="$SCRIPTS_DIR/$f1-$f2.sh"
      : > "$CUR_SCRIPT"
      ;;
    CMD)
      [ "$LANE_SELECTED" = 1 ] || continue
      printf '%s\n' "$f5" >> "$CUR_SCRIPT"
      ;;
  esac
done < "$STREAM"
finish_lane

if [ "$KEEP" = 1 ]; then
  printf '%s\n' "local-ci: --keep: step scripts in $SCRIPTS_DIR"
fi
printf '%s\n' "local-ci: all selected lanes passed"
