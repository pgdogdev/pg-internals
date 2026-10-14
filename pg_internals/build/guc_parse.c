/* Compile the upstream implementation directly. Rust's linker discards the
 * unused backend functions and data, retaining the parsers and unit tables.
 */
#include "guc_parse.h"
#include "utils/misc/guc.c"
