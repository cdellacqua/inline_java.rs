## TODO

avoid race conditions caused by thread handling same-hash java! invocations (wip)

avoid race conditions caused by thread handling same-hash ct_java! invocations

avoid recompilation of already-encountered source files (wip)

quotes around $arg in demo+tests

remove CARGO_MANIFEST_DIR from demo+tests

/tmp gets cleaned after a while, so write+compile again if necessary!

lock on file instead of mutex? would it work on windows? this would also allow share between processes and fix the /tmp missing file issue. would also fix the issue uniformly for ct_java as well!