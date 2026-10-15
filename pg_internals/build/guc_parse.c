/* The build script appends only the numeric parsers, their conversion helper,
 * and unit tables from the pinned upstream guc.c. */
#include "guc_parse.h"
#include "postgres.h"

#include <limits.h>
#include <math.h>

#include "utils/guc.h"
