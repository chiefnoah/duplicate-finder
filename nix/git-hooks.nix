{
  lib,
  package,
  threshold ? 0.8,
  minNodes ? 5,
}:
{
  duplicate-finder = {
    enable = true;
    name = "Duplicate finder";
    entry = "${lib.getExe package} . --threshold ${builtins.toString threshold} --min-nodes ${builtins.toString minNodes} --fail-on-clones";
    language = "unsupported";
    pass_filenames = false;
    always_run = true;
  };
}
