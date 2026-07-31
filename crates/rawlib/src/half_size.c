#include "libraw_types.h"

void libraw_set_half_size(libraw_data_t *lr, int value) {
    lr->params.half_size = value;
}

void libraw_set_use_camera_wb(libraw_data_t *lr, int value) {
    lr->params.use_camera_wb = value;
}
