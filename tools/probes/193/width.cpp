// Trailing whitespace on otherwise-blank lines was stripped when this file was
// committed, because the repository rejects it. No field, type, gate, member
// name, order or packing pragma was touched, so every width below is still
// measured from the verbatim struct bodies.

// Width probe for the item-window records. Runs on the host (x86-64), so
// `long` is 8 bytes here and 4 on the legacy i686 target. The substitution
// below is textual and audited: LONG is a 4-byte stand-in for `long`, and
// the control structs prove the substitution holds under #pragma pack(1).
#include <cstdint>
#include <cstdio>
#include <cstddef>

// gates copied from server/server/common/prodomodefines.h
#define __ATTR_6TH_7TH__ 1
#define __AURA_SYSTEM__ 1
#define ENABLE_SWITCHBOT 1
#define ENABLE_EXTENDED_SOCKETS 1
#define ENABLE_REFINE_ELEMENT 1
#define __CHANGELOOK_SYSTEM__ 1
#define __EXTENDED_SAFEBOX__ 1
enum EItemMisc
{
	ITEM_NAME_MAX_LEN			= 36,
	ITEM_VALUES_MAX_NUM			= 6,
	ITEM_SMALL_DESCR_MAX_LEN	= 256,
	ITEM_LIMIT_MAX_NUM			= 2,
	ITEM_APPLY_MAX_NUM			= 3,
#ifdef ENABLE_EXTENDED_SOCKETS
	ITEM_SOCKET_MAX_NUM			= 6,
	ITEM_STONES_MAX_NUM 		= 3, //If you are extending stones in item, change to 6. If you do not want more than 3 stones, keep 3.(git)
#else
	ITEM_SOCKET_MAX_NUM			= 3,
#endif
	ITEM_MAX_COUNT				= 5000,

	ITEM_ATTRIBUTE_NORM_NUM		= 5,
	ITEM_ATTRIBUTE_RARE_NUM		= 2,

	ITEM_ATTRIBUTE_NORM_START	= 0,
	ITEM_ATTRIBUTE_NORM_END		= ITEM_ATTRIBUTE_NORM_START + ITEM_ATTRIBUTE_NORM_NUM,

	ITEM_ATTRIBUTE_RARE_START	= ITEM_ATTRIBUTE_NORM_END,
	ITEM_ATTRIBUTE_RARE_END		= ITEM_ATTRIBUTE_RARE_START + ITEM_ATTRIBUTE_RARE_NUM,

	ITEM_ATTRIBUTE_MAX_NUM		= ITEM_ATTRIBUTE_RARE_END, // 7
#ifdef ENABLE_SWITCHBOT
	MAX_NORM_ATTR_NUM			= 5,
	MAX_RARE_ATTR_NUM			= 2,
#endif
	ITEM_ATTRIBUTE_MAX_LEVEL	= 5,
	ITEM_AWARD_WHY_MAX_LEN		= 50,

	REFINE_MATERIAL_MAX_NUM		= 5,

	ITEM_ELK_VNUM				= 50026,

	// ITEM_TOGGLE items:
	// Unique group, or -1 if unlimited simultaneously active items
	ITEM_VALUE_TOGGLE_GROUP = 5,

	ITEM_SOCKET_UNIQUE_SAVE_TIME = ITEM_SOCKET_MAX_NUM - 2,
	ITEM_SOCKET_UNIQUE_REMAIN_TIME = ITEM_SOCKET_MAX_NUM - 1,

	// ITEM_TOGGLE items
	ITEM_SOCKET_TOGGLE_TIME = 0,
	ITEM_SOCKET_TOGGLE_ACTIVE = 3,
	ITEM_SOCKET_TOGGLE_RIDING = 4,
};

#define long LONG
typedef int32_t LONG;

#pragma pack(push, 1)

typedef unsigned char BYTE;
typedef uint16_t WORD;
typedef uint32_t DWORD;
typedef bool BOOL_;

// ---- controls -----------------------------------------------------------
struct CtrlTwoLong { long a; long b; };          // must be 8 on i686
struct CtrlByteWordBool { BYTE a; WORD b; bool c; }; // 4 under pack(1)
struct CtrlPlain { DWORD a; DWORD b; DWORD c; };    // 12 under pack(1)

// ---- verbatim legacy types (body text copied unchanged) ------------------
typedef struct SItemPos
{
	BYTE window_type;
	WORD cell;
} TItemPos;

typedef struct TPlayerItemAttribute
{
	BYTE	bType;
	short	sValue;
} TPlayerItemAttribute;

// enum EWindows, verbatim member order, gates from prodomodefines.h
enum EWindows
{
	RESERVED_WINDOW,
	INVENTORY,
	EQUIPMENT,
	SAFEBOX,
	MALL,
	DRAGON_SOUL_INVENTORY,
#if defined(__ATTR_6TH_7TH__)
	ATTR67_ADD,
#endif
#ifdef __AURA_SYSTEM__
	AURA_REFINE,
#endif
#ifdef ENABLE_SWITCHBOT
	SWITCHBOT,
#endif
	BELT_INVENTORY,
	GROUND
};

// packet_item_set, verbatim field order, gates from prodomodefines.h
typedef struct packet_item_set
{
	BYTE	header;
	TItemPos Cell;
	DWORD	vnum;
	WORD	count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
	DWORD	flags;
	DWORD	anti_flags;
	bool	highlight;
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
} TPacketGCItemSet;

// packet_item_update, verbatim field order
typedef struct packet_item_update
{
	BYTE	header;
	TItemPos Cell;
	WORD	count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
} TPacketGCItemUpdate;

// packet_item_ground_add, verbatim field order
typedef struct packet_item_ground_add
{
	BYTE	bHeader;
	long 	x, y, z;
	DWORD	dwVID;
	DWORD	dwVnum;
} TPacketGCItemGroundAdd;

// packet_item_move, verbatim field order
struct TPacketGCItemDelDeprecated
{
	BYTE	header;
	TItemPos Cell;
	DWORD	vnum;
	BYTE	count;
#ifdef ENABLE_REFINE_ELEMENT
	DWORD	dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
	DWORD	transmutation;
#endif
	long	alSockets[ITEM_SOCKET_MAX_NUM];
	TPlayerItemAttribute aAttr[ITEM_ATTRIBUTE_MAX_NUM];
};

typedef struct packet_item_del
{
	BYTE	header;
#ifdef __EXTENDED_SAFEBOX__
	DWORD	pos;
#else
	BYTE	pos;
#endif
} TPacketGCItemDel;

struct packet_item_move
{
	BYTE	header;
	TItemPos Cell;
	TItemPos CellTo;
};

// ---- CG item records, verbatim field order, no `long` (host-trustworthy) --
typedef struct command_item_use { BYTE header; TItemPos Cell; } TPacketCGItemUse;
typedef struct command_item_drop { BYTE header; TItemPos Cell; DWORD gold; } TPacketCGItemDrop;
typedef struct command_item_drop2 { BYTE header; TItemPos Cell; DWORD gold; WORD count; } TPacketCGItemDrop2;
typedef struct command_item_destroy { BYTE header; TItemPos Cell; } TPacketCGItemDestroy;
typedef struct command_item_move { BYTE header; TItemPos Cell; TItemPos CellTo; WORD count; } TPacketCGItemMove;
typedef struct command_item_pickup { BYTE header; DWORD vid; } TPacketCGItemPickup;
typedef struct command_item_use_to_item { BYTE header; TItemPos Cell; TItemPos TargetCell; } TPacketCGItemUseToItem;
typedef struct packet_cg_give_item { BYTE byHeader; DWORD dwTargetVID; TItemPos ItemPos; BYTE byItemCount; } TPacketCGGiveItem;
typedef struct packet_cg_script_select_item { BYTE header; DWORD selection; } TPacketCGScriptSelectItem;
// the near-identical GC-side struct that has NO trailing count is the
// packet_item_move already defined above; see the note in the ledger.

#pragma pack(pop)

#define REPORT(T) printf("%-26s %3zu\n", #T, sizeof(T))
#define VALUE(E) printf("%-26s %3d\n", #E, (int)E)

int main()
{
	REPORT(CtrlTwoLong);
	REPORT(CtrlByteWordBool);
	REPORT(CtrlPlain);
	REPORT(TItemPos);
	REPORT(TPlayerItemAttribute);
	REPORT(TPacketGCItemSet);
	REPORT(TPacketGCItemUpdate);
	REPORT(TPacketGCItemGroundAdd);
	REPORT(packet_item_move);
	REPORT(TPacketCGItemUse); REPORT(TPacketCGItemDrop); REPORT(TPacketCGItemDrop2);
	REPORT(TPacketCGItemDestroy); REPORT(TPacketCGItemMove); REPORT(TPacketCGItemPickup);
	REPORT(TPacketCGItemUseToItem); REPORT(TPacketCGGiveItem); REPORT(TPacketCGScriptSelectItem);
	REPORT(packet_item_move);
	REPORT(TPacketGCItemDel);
	REPORT(TPacketGCItemDelDeprecated);
	printf("%-26s %3zu\n", "offsetof dep.Cell", offsetof(TPacketGCItemDelDeprecated, Cell));
	printf("%-26s %3zu\n", "offsetof dep.vnum", offsetof(TPacketGCItemDelDeprecated, vnum));
	printf("%-26s %3zu\n", "offsetof dep.count", offsetof(TPacketGCItemDelDeprecated, count));
	printf("%-26s %3zu\n", "offsetof dep.dwRefineElement", offsetof(TPacketGCItemDelDeprecated, dwRefineElement));
	printf("%-26s %3zu\n", "offsetof dep.transmutation", offsetof(TPacketGCItemDelDeprecated, transmutation));
	printf("%-26s %3zu\n", "offsetof dep.alSockets", offsetof(TPacketGCItemDelDeprecated, alSockets));
	printf("%-26s %3zu\n", "offsetof dep.aAttr", offsetof(TPacketGCItemDelDeprecated, aAttr));
	printf("%-26s %3zu\n", "offsetof del.pos", offsetof(TPacketGCItemDel, pos));
	printf("--- offets in TPacketGCItemUpdate ---\n");
	printf("--- offets in TPacketGCItemSet ---\n");
	printf("%-26s %3zu\n", "offsetof Cell", offsetof(TPacketGCItemSet, Cell));
	printf("%-26s %3zu\n", "offsetof vnum", offsetof(TPacketGCItemSet, vnum));
	printf("%-26s %3zu\n", "offsetof count", offsetof(TPacketGCItemSet, count));
	printf("%-26s %3zu\n", "offsetof dwRefineElement", offsetof(TPacketGCItemSet, dwRefineElement));
	printf("%-26s %3zu\n", "offsetof transmutation", offsetof(TPacketGCItemSet, transmutation));
	printf("%-26s %3zu\n", "offsetof flags", offsetof(TPacketGCItemSet, flags));
	printf("%-26s %3zu\n", "offsetof anti_flags", offsetof(TPacketGCItemSet, anti_flags));
	printf("%-26s %3zu\n", "offsetof highlight", offsetof(TPacketGCItemSet, highlight));
	printf("%-26s %3zu\n", "offsetof alSockets", offsetof(TPacketGCItemSet, alSockets));
	printf("%-26s %3zu\n", "offsetof aAttr", offsetof(TPacketGCItemSet, aAttr));
	printf("--- EWindows values ---\n");
	VALUE(RESERVED_WINDOW); VALUE(INVENTORY); VALUE(EQUIPMENT); VALUE(SAFEBOX);
	VALUE(MALL); VALUE(DRAGON_SOUL_INVENTORY); VALUE(ATTR67_ADD); VALUE(AURA_REFINE);
	VALUE(SWITCHBOT); VALUE(BELT_INVENTORY); VALUE(GROUND);
	printf("%-26s %3zu\n", "offsetof up.Cell", offsetof(TPacketGCItemUpdate, Cell));
	printf("%-26s %3zu\n", "offsetof up.count", offsetof(TPacketGCItemUpdate, count));
	printf("%-26s %3zu\n", "offsetof up.dwRefineElement", offsetof(TPacketGCItemUpdate, dwRefineElement));
	printf("%-26s %3zu\n", "offsetof up.transmutation", offsetof(TPacketGCItemUpdate, transmutation));
	printf("%-26s %3zu\n", "offsetof up.alSockets", offsetof(TPacketGCItemUpdate, alSockets));
	printf("%-26s %3zu\n", "offsetof up.aAttr", offsetof(TPacketGCItemUpdate, aAttr));
	printf("%-26s %3zu\n", "offsetof gadd.bHeader", offsetof(TPacketGCItemGroundAdd, bHeader));
	printf("%-26s %3zu\n", "offsetof gadd.x", offsetof(TPacketGCItemGroundAdd, x));
	printf("%-26s %3zu\n", "offsetof gadd.dwVID", offsetof(TPacketGCItemGroundAdd, dwVID));
	printf("ITEM_SOCKET_MAX_NUM     %3d\n", (int)ITEM_SOCKET_MAX_NUM);
	printf("ITEM_ATTRIBUTE_MAX_NUM  %3d\n", (int)ITEM_ATTRIBUTE_MAX_NUM);
	return 0;
}
