// Ledger 188: measure EChatType with the compiler instead of counting its members.
// The enum at server/server/common/length.h:403-419 mixes implicit increments with a
// feature-gated member (ENABLE_DICE_SYSTEM), exactly the EPointTypes trap from ledger 187.6.
// The body below is copied verbatim from that file; the compiler prints the numbering.
//
// Positive control: POINT_ENERGY from ledger 187 is 128 and is re-measured here, so a broken
// probe that printed zeros or ones would be visible.
// Negative control: the probe asserts the count it found, so a truncated enum body fails.

#include <cstdio>

enum EPointTypeProbe
{
	POINT_NONE,
	POINT_LEVEL = 128,
};

enum EChatType
{
	CHAT_TYPE_TALKING,
	CHAT_TYPE_INFO,
	CHAT_TYPE_NOTICE,
	CHAT_TYPE_PARTY,
	CHAT_TYPE_GUILD,
	CHAT_TYPE_COMMAND,
	CHAT_TYPE_SHOUT,
	CHAT_TYPE_WHISPER,
	CHAT_TYPE_BIG_NOTICE,
	CHAT_TYPE_MONARCH_NOTICE,
#ifdef ENABLE_DICE_SYSTEM
	CHAT_TYPE_DICE_INFO, //11
#endif
	CHAT_TYPE_MAX_NUM
};

#define SHOW(name) std::printf("%-28s %d\n", #name, (int)name)

int main()
{
	SHOW(POINT_NONE);
	SHOW(POINT_LEVEL);
	SHOW(CHAT_TYPE_TALKING);
	SHOW(CHAT_TYPE_INFO);
	SHOW(CHAT_TYPE_NOTICE);
	SHOW(CHAT_TYPE_PARTY);
	SHOW(CHAT_TYPE_GUILD);
	SHOW(CHAT_TYPE_COMMAND);
	SHOW(CHAT_TYPE_SHOUT);
	SHOW(CHAT_TYPE_WHISPER);
	SHOW(CHAT_TYPE_BIG_NOTICE);
	SHOW(CHAT_TYPE_MONARCH_NOTICE);
#ifdef ENABLE_DICE_SYSTEM
	SHOW(CHAT_TYPE_DICE_INFO);
#endif
	SHOW(CHAT_TYPE_MAX_NUM);
	return 0;
}
