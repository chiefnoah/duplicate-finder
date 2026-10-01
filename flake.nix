{
  description = "Duplicate Finder development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    git-hooks.url = "github:cachix/git-hooks.nix";
  };

  outputs = { self, nixpkgs, git-hooks, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
    in
    {
      packages = nixpkgs.lib.genAttrs systems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.callPackage ./nix/package.nix { };
          df = pkgs.callPackage ./nix/package.nix { };
        });

      checks = nixpkgs.lib.genAttrs systems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          pre-commit-check = git-hooks.lib.${system}.run {
            src = ./.;
            hooks.rustfmt.enable = true;
          };
        });

      devShells = nixpkgs.lib.genAttrs systems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          pre-commit-check = self.checks.${system}.pre-commit-check;
          target = pkgs.lib.toUpper (builtins.replaceStrings [ "-" ] [ "_" ] pkgs.stdenv.hostPlatform.rust.rustcTarget);
          compiler = "${pkgs.stdenv.cc}/bin/cc";
        in
        {
          default = pkgs.mkShell {
            inherit (pre-commit-check) shellHook;
            nativeBuildInputs = with pkgs; [ cargo rustc rustfmt clippy ] ++ pre-commit-check.enabledPackages;
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];

            # Use the shell compiler for Rust and tree-sitter C grammars.
            CC = compiler;
            "CARGO_TARGET_${target}_LINKER" = compiler;
          };
        });
    };
}
