typedef unsigned char BYTE;
typedef unsigned short WORD;
typedef unsigned int DWORD;
typedef long LONG;
#pragma pack(1)
struct TwoLong { unsigned long a; unsigned long b; };
typedef struct
{
	DWORD	dwID;
	long	x, y;
	long	width, height;
	DWORD	dwGuildID;
} TLandPacketElement;
typedef struct packet_land_list
{
	BYTE	header;
	WORD	size;
} TPacketGCLandList;
#pragma pack()
static_assert(sizeof(TwoLong) == 8, "two-long control");
static_assert(sizeof(TLandPacketElement) == 24, "element");
static_assert(sizeof(TPacketGCLandList) == 3, "header");
int main() { return 0; }
