default: check-all

check-all: 
  #!/bin/bash
  DIRS="$(find . -maxdepth 2 -type f -name Cargo.toml -exec dirname {} \;)"
  for dir in $DIRS; do 
    echo "[$dir] cargo clippy"
    (cd "$dir" && cargo clippy)
  done

