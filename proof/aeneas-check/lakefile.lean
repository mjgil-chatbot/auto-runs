import Lake
open Lake DSL

require aeneas from "../../../others/aeneas-tool/backends/lean"

package «auto-runs-aeneas-check» {}

@[default_target]
lean_lib «AutoRunsExtract»
