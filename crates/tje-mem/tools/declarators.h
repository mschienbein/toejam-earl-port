/* Synthetic declarations: multiplication belongs to bounds, not pointer prefixes. */
#include "terrain.h"
extern unsigned char multiplied[MAP_ROWS * 2][MAP_COLUMNS];
extern unsigned char ***three_levels;
extern unsigned char **pointer_rows[3 * 2][5];
extern unsigned short incomplete[][3 * 4];
extern char *left, **right[2 * 3];
extern void (*callback)();
