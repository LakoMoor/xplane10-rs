#include <ft2build.h>
#include FT_FREETYPE_H
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
/* ftprobe <ttf> <size> <flags hex> <text> [pen rounding 0|1] : coverage grid 24x120, baseline row 16 */
int main(int c, char **v) {
    FT_Library lib; FT_Face face; FT_Init_FreeType(&lib);
    if (FT_New_Face(lib, v[1], 0, &face)) return 1;
    FT_Set_Char_Size(face, 0, (long)(atof(v[2]) * 64.0), 72, 72);
    long flags = strtol(v[3], 0, 16);
    static unsigned char g[24][120];
    double pen = 0; int rnd = c > 5 ? atoi(v[5]) : 1;
    for (const char *p = v[4]; *p; p++) {
        if (FT_Load_Char(face, (unsigned char)*p, (FT_Int32)flags)) continue;
        FT_Render_Glyph(face->glyph, FT_RENDER_MODE_NORMAL);
        FT_GlyphSlot s = face->glyph;
        int x0 = (int)(rnd ? pen + 0.5 : pen) + s->bitmap_left, y0 = 16 - s->bitmap_top;
        for (unsigned y = 0; y < s->bitmap.rows; y++) for (unsigned x = 0; x < s->bitmap.width; x++) {
            int gx = x0 + (int)x, gy = y0 + (int)y;
            if (gx >= 0 && gy >= 0 && gy < 24 && gx < 120) { int t = g[gy][gx] + s->bitmap.buffer[y * s->bitmap.pitch + x]; g[gy][gx] = t > 255 ? 255 : t; }
        }
        printf("# %c adv %.4f left %d\n", *p, s->advance.x / 64.0, s->bitmap_left);
        pen += s->advance.x / 64.0;
    }
    for (int y = 0; y < 24; y++) { for (int x = 0; x < 120; x++) printf("%d ", g[y][x]); printf("\n"); }
    int maj, min, pat; FT_Library_Version(lib, &maj, &min, &pat); fprintf(stderr, "freetype %d.%d.%d\n", maj, min, pat);
    return 0;
}
