import Lake
open Lake DSL

package «auto-runs-proof» where
  leanOptions := #[
    ⟨`autoImplicit, false⟩
  ]

@[default_target]
lean_lib AutoRunsProof where
  srcDir := "."
