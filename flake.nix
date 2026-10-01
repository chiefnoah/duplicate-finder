{
  description = "Duplicate Finder development shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs = { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
    in
    {
      devShells = nixpkgs.lib.genAttrs systems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          target = pkgs.lib.toUpper (builtins.replaceStrings [ "-" ] [ "_" ] pkgs.stdenv.hostPlatform.rust.rustcTarget);
          compiler = "${pkgs.stdenv.cc}/bin/cc";
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [ cargo rustc rustfmt clippy ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];

            # Use the shell compiler for Rust and tree-sitter C grammars.
            CC = compiler;
            "CARGO_TARGET_${target}_LINKER" = compiler;
          };
        });
    };
}
