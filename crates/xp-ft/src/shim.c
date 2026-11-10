/* A flat C surface over FreeType, so the Rust side never touches FT_FaceRec's layout. */
#include <ft2build.h>
#include FT_FREETYPE_H
#include <stdlib.h>
#include <string.h>

typedef struct { FT_Library lib; FT_Face face; } xpft_face;

typedef struct {
    int width, rows, left, top;
    double advance;
} xpft_glyph;

/* size is in points at 72 dpi, i.e. pixels: FT_Set_Char_Size(face, 0, size * 64, 72, 72),
   which is the call OGL_face_info::load makes (oracle 0x1000a5dc0). */
void *xpft_open(const char *path, double size) {
    xpft_face *f = calloc(1, sizeof *f);
    if (!f) return NULL;
    if (FT_Init_FreeType(&f->lib)) { free(f); return NULL; }
    if (FT_New_Face(f->lib, path, 0, &f->face)) { FT_Done_FreeType(f->lib); free(f); return NULL; }
    if (FT_Set_Char_Size(f->face, 0, (FT_F26Dot6)(size * 64.0), 72, 72)) {
        FT_Done_Face(f->face); FT_Done_FreeType(f->lib); free(f); return NULL;
    }
    return f;
}

void xpft_close(void *h) {
    xpft_face *f = h;
    if (!f) return;
    FT_Done_Face(f->face);
    FT_Done_FreeType(f->lib);
    free(f);
}

/* Loads with the given FT_LOAD_* flags, renders 8-bit coverage, copies it row by row into buf.
   Returns 0 on success, 1 on a FreeType error, 2 when buf is too small. */
int xpft_glyph_render(void *h, unsigned long ch, int flags, xpft_glyph *out, unsigned char *buf, int cap) {
    xpft_face *f = h;
    if (FT_Load_Char(f->face, ch, flags)) return 1;
    if (FT_Render_Glyph(f->face->glyph, FT_RENDER_MODE_NORMAL)) return 1;
    FT_GlyphSlot s = f->face->glyph;
    out->width = (int)s->bitmap.width;
    out->rows = (int)s->bitmap.rows;
    out->left = s->bitmap_left;
    out->top = s->bitmap_top;
    out->advance = s->advance.x / 64.0;
    if (out->width * out->rows > cap) return 2;
    for (int y = 0; y < out->rows; y++)
        memcpy(buf + y * out->width, s->bitmap.buffer + y * s->bitmap.pitch, (size_t)out->width);
    return 0;
}

void xpft_version(int *maj, int *min, int *pat) {
    FT_Library lib;
    if (FT_Init_FreeType(&lib)) { *maj = *min = *pat = 0; return; }
    FT_Library_Version(lib, maj, min, pat);
    FT_Done_FreeType(lib);
}
