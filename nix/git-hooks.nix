{ lib, package }:
{
  duplicate-finder = {
    enable = true;
    name = "Duplicate finder";
    entry = "${lib.getExe package} . --fail-on-clones";
    language = "unsupported";
    pass_filenames = false;
    always_run = true;
  };
}
