#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

source_roots=(
  crates/benilla-ui/src/script
  crates/benilla-ui/src/loader
)
source_globs=(
  --glob '!**/tests/**'
  --glob '!**/tests.rs'
)

inventory_file=$(mktemp)
trap 'rm -f "$inventory_file"' EXIT

rg "${source_globs[@]}" -l '\bmlua(?:::|\b)' "${source_roots[@]}" | sort > "$inventory_file"

file_count=$(wc -l < "$inventory_file" | tr -d ' ')
occurrence_count=$(
  rg "${source_globs[@]}" -o '\bmlua(?:::|\b)' "${source_roots[@]}" | wc -l | tr -d ' '
)

printf '# Production Lua host inventory\n\n'
printf -- '- files naming `mlua`: %s\n' "$file_count"
printf -- '- direct `mlua` occurrences: %s\n\n' "$occurrence_count"

printf '## VM capabilities\n\n'
printf '| Capability | Files | Occurrences |\n'
printf '| --- | ---: | ---: |\n'

capability() {
  local name=$1
  local pattern=$2
  local matches
  local files
  local occurrences

  matches=$(rg "${source_globs[@]}" -n -e "$pattern" "${source_roots[@]}" || true)
  if [[ -z $matches ]]; then
    files=0
    occurrences=0
  else
    files=$(printf '%s\n' "$matches" | cut -d: -f1 | sort -u | wc -l | tr -d ' ')
    occurrences=$(printf '%s\n' "$matches" | wc -l | tr -d ' ')
  fi
  printf '| %s | %s | %s |\n' "$name" "$files" "$occurrences"
}

capability 'VM/chunks' 'Lua::new|Lua::new_with|\.load\(|\.exec\(|ChunkMode'
capability 'Values/conversions' '\b(Value|MultiValue|Variadic|FromLua|FromLuaMulti|IntoLua|IntoLuaMulti)\b'
capability 'Tables/metatables' '\bTable\b|ObjectLike|create_table|globals\(|raw_(get|set)|set_metatable|pairs::<'
capability 'Functions/calls' '\bFunction\b|call_method|\.call::<|\.call\('
capability 'Captured callbacks' 'create_function|create_c_function'
capability 'Registry/roots' 'named_registry|registry_value|RegistryKey|set_named_registry'
capability 'Typed app data' 'app_data|AppData'
capability 'Errors/tracebacks' 'mlua::(Error|Result)|\bVmState\b|Error::runtime'
capability 'Hooks/budget' 'set_hook|HookTriggers|mlua::Debug'
capability 'Userdata/identity' 'UserData|LightUserData|create_userdata|AnyUserData'

declare -A layers=(
  [loader]=0
  [object_model]=0
  [runtime]=0
  [gameplay_api]=0
)

while IFS= read -r file; do
  base=${file##*/}
  if [[ $file == */loader/* ]]; then
    layer=loader
  elif [[ $file == */script/object/* ]] ||
    [[ $base =~ ^(backdrop|button|clip|colorselect|editbox|font|font_block|layout|measure|model|modelframe|region|scrollframe|slider|statusbar)\.rs$ ]]; then
    layer=object_model
  elif [[ $base =~ ^(addon|addon_gate|diagnostics|event|handler_prof|instance|keyboard|mod|pointer|saved|stdlib|surface|tick)\.rs$ ]]; then
    layer=runtime
  else
    layer=gameplay_api
  fi
  layers[$layer]=$((layers[$layer] + 1))
done < "$inventory_file"

printf '\n## UI layers\n\n'
printf '| Layer | Files |\n'
printf '| --- | ---: |\n'
printf '| Loader | %s |\n' "${layers[loader]}"
printf '| Object/widget model | %s |\n' "${layers[object_model]}"
printf '| VM/runtime lifecycle | %s |\n' "${layers[runtime]}"
printf '| Gameplay API families | %s |\n' "${layers[gameplay_api]}"

assigned=$((layers[loader] + layers[object_model] + layers[runtime] + layers[gameplay_api]))
printf '\nAssigned files: %s/%s\n' "$assigned" "$file_count"
if [[ $assigned -ne $file_count ]]; then
  exit 1
fi
