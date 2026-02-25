#ifndef __INC_LIBTHECORE_BUFFER_H__
#define __INC_LIBTHECORE_BUFFER_H__

#define SAFE_BUFFER_DELETE(buf)		{ if(buf != NULL) { buffer_delete(buf); buf = NULL; } }

typedef struct buffer BUFFER;
typedef struct buffer* LPBUFFER;

struct buffer
{
	struct buffer* next;

	char* write_point;
	int32_t write_point_pos;

	const char* read_point;
	int32_t length;

	char* mem_data;
	int32_t mem_size;

	int32_t flag;
};

extern LPBUFFER buffer_new(int32_t size);
extern void buffer_delete(LPBUFFER buffer);
extern void buffer_reset(LPBUFFER buffer);

extern uint32_t buffer_size(LPBUFFER buffer);
extern int32_t buffer_has_space(LPBUFFER buffer);

extern void buffer_write(LPBUFFER& buffer, const void* src, int32_t length);
extern void buffer_read(LPBUFFER buffer, void* buf, int32_t bytes);
extern uint8_t buffer_get_byte(LPBUFFER buffer);
extern uint16_t buffer_get_word(LPBUFFER buffer);
extern uint32_t buffer_get_dword(LPBUFFER buffer);

extern const void* buffer_read_peek(LPBUFFER buffer);
extern void buffer_read_proceed(LPBUFFER buffer, int32_t length);

extern void* buffer_write_peek(LPBUFFER buffer);
extern void buffer_write_proceed(LPBUFFER buffer, int32_t length);

extern void buffer_adjust_size(LPBUFFER& buffer, int32_t add_size);
#endif
