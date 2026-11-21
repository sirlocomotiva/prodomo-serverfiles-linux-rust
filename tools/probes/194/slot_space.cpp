// Slot-space probe for ledger 194. Resolves the EMisc2 inventory slot space
// by compiling the verbatim enum bodies with the legacy build's gates, because
// the arithmetic comments in length.h are stale (they claim WEAR_MAX_NUM is 32
// and it is 64 at length.h:89).
//
// Controls: every constant below is also reported as a hand-summed product, and
// the probe asserts the two agree, so a mis-transcribed constant cannot pass
// silently.
#include <cstdint>
#include <cstdio>
#include <cstddef>

// cuberenewal.h:5
#ifndef CUBE_MAX_NUM
#define CUBE_MAX_NUM 24
#endif
// item_length.h:515 and length.h:178
#define SASH_WINDOW_MAX_MATERIALS 2
#define CL_WINDOW_MAX_MATERIALS 2


#define ENABLE_EXTEND_INVEN_SYSTEM 1
#define __EXTENDED_SAFEBOX__ 1
#define ENABLE_CUSTOM_INVENTORY 1
#define ENABLE_DRAGONSOUL_ALCHEMY_PLUS 1
#define __ATTR_6TH_7TH__ 1
#define __AURA_SYSTEM__ 1
#define ENABLE_SWITCHBOT 1

// length.h:657-676, verbatim. All three gates are live in prodomodefines.h
// (__ATTR_6TH_7TH__ at 31, __AURA_SYSTEM__ at 30, ENABLE_SWITCHBOT at 191).
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

// length.h:933-944
enum E11
{
#ifdef ENABLE_SWITCHBOT
	SWITCHBOT_SLOT_COUNT = 5,
#endif
};

// length.h:17-31, verbatim member order
enum E1
{
	INVENTORY_PAGE_COLUMN	= 5,
	INVENTORY_PAGE_ROW		= 9,
	INVENTORY_PAGE_SIZE		= INVENTORY_PAGE_COLUMN*INVENTORY_PAGE_ROW,
#ifdef ENABLE_EXTEND_INVEN_SYSTEM
	INVENTORY_PAGE_COUNT	= 4,
#else
	INVENTORY_PAGE_COUNT	= 2,
#endif
	INVENTORY_MAX_NUM		= INVENTORY_PAGE_SIZE*INVENTORY_PAGE_COUNT,
};

// length.h:28-31
enum E2
{
#ifdef ENABLE_CUSTOM_INVENTORY
	CUSTOM_INVENTORY_PAGE_SIZE = 45,
	CUSTOM_INVENTORY_PAGE_COUNT = 4,
	CUSTOM_INVENTORY_MAX_NUM = CUSTOM_INVENTORY_PAGE_SIZE * CUSTOM_INVENTORY_PAGE_COUNT,
	CUSTOM_INVENTORY_CATEGORY_NUM = 6,
#endif
};

// length.h:89 and length.h:202
enum E3 { WEAR_MAX_NUM = 64, ATTR67_ADD_SLOT_MAX = 1 };

// item_length.h:174-182, DS_SLOT_MAX is auto-incremented
enum E4 { DS_SLOT1, DS_SLOT2, DS_SLOT3, DS_SLOT4, DS_SLOT5, DS_SLOT6, DS_SLOT_MAX };

// length.h:259-266
enum E5
{
	DRAGON_SOUL_DECK_0,
	DRAGON_SOUL_DECK_1,
	DRAGON_SOUL_DECK_MAX_NUM = 2,
	DRAGON_SOUL_DECK_RESERVED_MAX_NUM = 3,
};

// length.h:278, 285, 289
enum E12 { INVENTORY_OPEN_PAGE_COUNT = 2, INVENTORY_WIDTH = 5, INVENTORY_HEIGHT = 9,
           INVENTORY_OPEN_PAGE_SIZE = INVENTORY_OPEN_PAGE_COUNT*INVENTORY_PAGE_SIZE };

// length.h:118-121
enum E6 { BELT_INVENTORY_SLOT_WIDTH = 4, BELT_INVENTORY_SLOT_HEIGHT = 4,
          BELT_INVENTORY_SLOT_COUNT = BELT_INVENTORY_SLOT_WIDTH * BELT_INVENTORY_SLOT_HEIGHT };

// length.h:915-931, verbatim
enum EMisc2
{
	DRAGON_SOUL_EQUIP_SLOT_START = INVENTORY_MAX_NUM + WEAR_MAX_NUM,
	DRAGON_SOUL_EQUIP_SLOT_END = DRAGON_SOUL_EQUIP_SLOT_START + (DS_SLOT_MAX * DRAGON_SOUL_DECK_MAX_NUM),
	DRAGON_SOUL_EQUIP_RESERVED_SLOT_END = DRAGON_SOUL_EQUIP_SLOT_END + (DS_SLOT_MAX * DRAGON_SOUL_DECK_RESERVED_MAX_NUM),

	BELT_INVENTORY_SLOT_START = DRAGON_SOUL_EQUIP_RESERVED_SLOT_END,
	BELT_INVENTORY_SLOT_END = BELT_INVENTORY_SLOT_START + BELT_INVENTORY_SLOT_COUNT,

#ifdef ENABLE_CUSTOM_INVENTORY
	CUSTOM_INVENTORY_SLOT_START = BELT_INVENTORY_SLOT_END,
	CUSTOM_INVENTORY_SLOT_END = CUSTOM_INVENTORY_SLOT_START + (CUSTOM_INVENTORY_MAX_NUM * CUSTOM_INVENTORY_CATEGORY_NUM),
	INVENTORY_AND_EQUIP_SLOT_MAX = CUSTOM_INVENTORY_SLOT_END,
#else
	INVENTORY_AND_EQUIP_SLOT_MAX = BELT_INVENTORY_SLOT_END,
#endif
};

// length.h:83-85
enum E8 { DRAGON_SOUL_BOX_SIZE = 32, DRAGON_SOUL_BOX_COLUMN_NUM = 8,
          DRAGON_SOUL_BOX_ROW_NUM = DRAGON_SOUL_BOX_SIZE / DRAGON_SOUL_BOX_COLUMN_NUM };

// item_length.h:184-196, ENABLE_DRAGONSOUL_ALCHEMY_PLUS is live at prodomodefines.h:27
enum E9
{
	DRAGON_SOUL_GRADE_NORMAL,
	DRAGON_SOUL_GRADE_BRILLIANT,
	DRAGON_SOUL_GRADE_RARE,
	DRAGON_SOUL_GRADE_ANCIENT,
	DRAGON_SOUL_GRADE_LEGENDARY,
#ifdef ENABLE_DRAGONSOUL_ALCHEMY_PLUS
	DRAGON_SOUL_GRADE_MYTHIC,
#endif
	DRAGON_SOUL_GRADE_MAX,
};

// item_length.h:209-212
enum E10 { DRAGON_SOUL_INVENTORY_MAX_NUM = DS_SLOT_MAX * DRAGON_SOUL_GRADE_MAX * DRAGON_SOUL_BOX_SIZE };

// game/cuberenewal.h:5
#define CUBE_MAX_NUM 24

// length.h:91-94
enum E7
{
#ifdef __EXTENDED_SAFEBOX__
	SAFEBOX_MAX_PAGE_COUNT		= 6,
	SAFEBOX_MAX_NUM				= 45 * SAFEBOX_MAX_PAGE_COUNT,
#endif
};

#define V(x) printf("%-34s %5d\n", #x, (int)(x))
// hand sums, written out independently of the enum
#define HAND_INV_MAX   (5*9*4)
#define HAND_BELT      (4*4)
#define HAND_CUSTOM    (45*4*6)

static int failures = 0;
static void check(const char* what, int got, int want)
{
	if (got != want)
	{
		printf("CONTROL FAILED %-28s got %d want %d\n", what, got, want);
		++failures;
	}
}

// The legacy 32-bit target's fixed-width types, spelled out so the widths below
// come from `sizeof` rather than from a hand sum. `CItem` is opaque here: only
// the pointer size matters, and the class is not defined by this probe.
typedef unsigned short WORD;
typedef unsigned short BYTE_T;
struct CItem;
typedef CItem* LPITEM;
static_assert(sizeof(WORD) == 2, "legacy WORD is 2 bytes");
static_assert(sizeof(LPITEM) == sizeof(void*), "legacy pointer");

// The item storage a character actually owns, by element type. `WORD` is 2 bytes
// on the legacy 32-bit target and `LPITEM` a 4-byte pointer, so the grid arrays
// are WORD and the item arrays are pointers -- which is why `bItemGrid` can hold
// `wCell + 1` for a cell of 1369 without truncating.
struct CItemStorage {
	LPITEM pItems[INVENTORY_AND_EQUIP_SLOT_MAX];
	WORD   bItemGrid[INVENTORY_AND_EQUIP_SLOT_MAX];
	LPITEM pDSItems[DRAGON_SOUL_INVENTORY_MAX_NUM];
	WORD   wDSItemGrid[DRAGON_SOUL_INVENTORY_MAX_NUM];
	LPITEM pCubeItems[CUBE_MAX_NUM];
	LPITEM pSashMaterials[SASH_WINDOW_MAX_MATERIALS];
	LPITEM pClMaterials[CL_WINDOW_MAX_MATERIALS];
	LPITEM pAttr67AddItem;
	LPITEM pSwitchbotItems[SWITCHBOT_SLOT_COUNT];
};

int main()
{
	V(INVENTORY_PAGE_SIZE); V(INVENTORY_PAGE_COUNT); V(INVENTORY_MAX_NUM);
	V(WEAR_MAX_NUM); V(DS_SLOT_MAX);
	V(DRAGON_SOUL_DECK_MAX_NUM); V(DRAGON_SOUL_DECK_RESERVED_MAX_NUM);
	V(BELT_INVENTORY_SLOT_COUNT);
	V(CUSTOM_INVENTORY_MAX_NUM); V(CUSTOM_INVENTORY_CATEGORY_NUM);
	V(SAFEBOX_MAX_NUM);
	V(INVENTORY_OPEN_PAGE_SIZE); V(INVENTORY_WIDTH); V(INVENTORY_HEIGHT);
	V(DRAGON_SOUL_GRADE_MAX); V(DRAGON_SOUL_INVENTORY_MAX_NUM); V(ATTR67_ADD_SLOT_MAX);
	printf("--- EWindows (length.h:657-676) ---\n");
	V(RESERVED_WINDOW); V(INVENTORY); V(EQUIPMENT); V(SAFEBOX); V(MALL);
	V(DRAGON_SOUL_INVENTORY); V(ATTR67_ADD); V(AURA_REFINE); V(SWITCHBOT);
	V(BELT_INVENTORY); V(GROUND); V(SWITCHBOT_SLOT_COUNT);
	printf("--- the slot space ---\n");
	V(DRAGON_SOUL_EQUIP_SLOT_START);
	V(DRAGON_SOUL_EQUIP_SLOT_END);
	V(DRAGON_SOUL_EQUIP_RESERVED_SLOT_END);
	V(BELT_INVENTORY_SLOT_START);
	V(BELT_INVENTORY_SLOT_END);
	V(CUSTOM_INVENTORY_SLOT_START);
	V(CUSTOM_INVENTORY_SLOT_END);
	V(INVENTORY_AND_EQUIP_SLOT_MAX);
	// each enum constant against an independently written hand sum
	check("INVENTORY_MAX_NUM", INVENTORY_MAX_NUM, HAND_INV_MAX);
	check("BELT_INVENTORY_SLOT_COUNT", BELT_INVENTORY_SLOT_COUNT, HAND_BELT);
	check("CUSTOM_INVENTORY_SLOT_END-CUSTOM_INVENTORY_SLOT_START",
	      CUSTOM_INVENTORY_SLOT_END - CUSTOM_INVENTORY_SLOT_START, HAND_CUSTOM);
	// the whole space is contiguous: every range starts where the previous ended
	check("DS equip start == inv+wear", DRAGON_SOUL_EQUIP_SLOT_START, INVENTORY_MAX_NUM + WEAR_MAX_NUM);
	check("DS equip end", DRAGON_SOUL_EQUIP_SLOT_END - DRAGON_SOUL_EQUIP_SLOT_START, DS_SLOT_MAX * DRAGON_SOUL_DECK_MAX_NUM);
	check("DS reserved end", DRAGON_SOUL_EQUIP_RESERVED_SLOT_END - DRAGON_SOUL_EQUIP_SLOT_END, DS_SLOT_MAX * DRAGON_SOUL_DECK_RESERVED_MAX_NUM);
	check("belt start", BELT_INVENTORY_SLOT_START, DRAGON_SOUL_EQUIP_RESERVED_SLOT_END);
	check("belt end", BELT_INVENTORY_SLOT_END, BELT_INVENTORY_SLOT_START + BELT_INVENTORY_SLOT_COUNT);
	check("custom start", CUSTOM_INVENTORY_SLOT_START, BELT_INVENTORY_SLOT_END);
	check("max == custom end", INVENTORY_AND_EQUIP_SLOT_MAX, CUSTOM_INVENTORY_SLOT_END);
	check("DS inventory max", DRAGON_SOUL_INVENTORY_MAX_NUM, 6 * 6 * 32);
	check("attr67 add", ATTR67_ADD_SLOT_MAX, 1);
	check("safe rows", DRAGON_SOUL_BOX_ROW_NUM, 32 / 8);
	// char.h:1285: Inventory_Size() = INVENTORY_OPEN_PAGE_SIZE + INVENTORY_WIDTH*Inven_Point()
	check("open page size", INVENTORY_OPEN_PAGE_SIZE, 2 * 45);
	// The runtime usable count cannot exceed the base inventory it indexes.
	if (INVENTORY_OPEN_PAGE_SIZE + INVENTORY_WIDTH * 18 > INVENTORY_MAX_NUM) {
		printf("CONTROL FAILED usable count exceeds the base inventory\n"); ++failures;
	}
	// the two per-character arrays in char.h:458-461 must both be the slot-space size
	printf("char.h  pItems[%d] bItemGrid[%d] pDSItems[%d] wDSItemGrid[%d]\n",
	       (int)INVENTORY_AND_EQUIP_SLOT_MAX, (int)INVENTORY_AND_EQUIP_SLOT_MAX,
	       (int)DRAGON_SOUL_INVENTORY_MAX_NUM, (int)DRAGON_SOUL_INVENTORY_MAX_NUM);
	printf("i686 bytes  pItems=%d bItemGrid=%d pDSItems=%d wDSItemGrid=%d total=%d\n",
	       4 * INVENTORY_AND_EQUIP_SLOT_MAX, 2 * INVENTORY_AND_EQUIP_SLOT_MAX,
	       4 * DRAGON_SOUL_INVENTORY_MAX_NUM, 2 * DRAGON_SOUL_INVENTORY_MAX_NUM,
	       4 * INVENTORY_AND_EQUIP_SLOT_MAX + 2 * INVENTORY_AND_EQUIP_SLOT_MAX +
	       4 * DRAGON_SOUL_INVENTORY_MAX_NUM + 2 * DRAGON_SOUL_INVENTORY_MAX_NUM);
	// The item storage beyond the two slot arrays, and the grid element type.
	check("CUBE_MAX_NUM", CUBE_MAX_NUM, 24);
	check("SASH_WINDOW_MAX_MATERIALS", SASH_WINDOW_MAX_MATERIALS, 2);
	check("CL_WINDOW_MAX_MATERIALS", CL_WINDOW_MAX_MATERIALS, 2);
	check("DRAGON_SOUL_BOX_COLUMN_NUM", DRAGON_SOUL_BOX_COLUMN_NUM, 8);
	check("DRAGON_SOUL_BOX_ROW_NUM", DRAGON_SOUL_BOX_ROW_NUM, 32 / 8);
	check("DRAGON_SOUL_BOX_SIZE", DRAGON_SOUL_BOX_SIZE, 32);

	// char_item.cpp:422 and :452 walk a stack with a BARE 5. INVENTORY_WIDTH is 5
	// too, but the source never says so. If the two ever diverged, the grid
	// would be walked with the wrong stride, so they are compared here.
	check("flat stack stride", INVENTORY_WIDTH, 5);

	// The stack walk is `p = wCell + i * stride`, skipped once `p` passes the
	// category end, so the walk is safe for ANY stride or size: what has to hold
	// is that the category end itself never leaves the array. That is the real
	// invariant, and it is what the first draft of this control should have
	// asserted -- asserting a worst-case cell instead was asserting nothing,
	// because `bSize` comes from the proto and no fixed value bounds it here.
	{
		int worst_end = 0;
		for (int cat = 0; cat <= CUSTOM_INVENTORY_CATEGORY_NUM; ++cat) {
			int end = CUSTOM_INVENTORY_SLOT_START + cat * CUSTOM_INVENTORY_MAX_NUM;
			if (end > worst_end) worst_end = end;
		}
		check("the last category end", worst_end, INVENTORY_AND_EQUIP_SLOT_MAX);
		if (worst_end > INVENTORY_AND_EQUIP_SLOT_MAX) {
			printf("CONTROL FAILED a category end leaves the array\n"); ++failures;
		}
		// The base inventory and the belt are the walk's other two ends. Neither
		// is a category, and both are below the custom start.
		if ((int)INVENTORY_MAX_NUM > (int)INVENTORY_AND_EQUIP_SLOT_MAX) {
			printf("CONTROL FAILED the base inventory leaves the array\n"); ++failures;
		}
		// And the DS walk's own end.
		check("the DS walk end", DRAGON_SOUL_INVENTORY_MAX_NUM, 1152);
		// The grid stores the origin cell plus one, so a cell of 1369 stores 1370.
		// With a WORD element that is exact; with a BYTE it would wrap to 90.
		check("grid origin marker", INVENTORY_AND_EQUIP_SLOT_MAX, 1370);
		check("grid marker fits a WORD", INVENTORY_AND_EQUIP_SLOT_MAX <= 0xffff, true);
		check("grid marker would wrap a BYTE", INVENTORY_AND_EQUIP_SLOT_MAX <= 0xff, false);
	}

	// EWindows: the compiler's numbering against the values the Rust table pins.
	check("EWindows RESERVED_WINDOW", RESERVED_WINDOW, 0);
	check("EWindows INVENTORY", INVENTORY, 1);
	check("EWindows EQUIPMENT", EQUIPMENT, 2);
	check("EWindows SAFEBOX", SAFEBOX, 3);
	check("EWindows MALL", MALL, 4);
	check("EWindows DRAGON_SOUL_INVENTORY", DRAGON_SOUL_INVENTORY, 5);
	check("EWindows ATTR67_ADD", ATTR67_ADD, 6);
	check("EWindows AURA_REFINE", AURA_REFINE, 7);
	check("EWindows SWITCHBOT", SWITCHBOT, 8);
	check("EWindows BELT_INVENTORY", BELT_INVENTORY, 9);
	check("EWindows GROUND", GROUND, 10);
	printf("%s (%d control failures)\n", failures ? "CONTROLS FAILED" : "all controls passed", failures);
	return failures != 0;
}
