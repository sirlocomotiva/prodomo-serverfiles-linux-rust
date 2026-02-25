#pragma once
#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
#include "../common/tables.h"

enum BiologistConfig
{
	REWARD_BONUS_MAX_COUNT = 3,
	MAX_MISSION_STATE = 9,
	DROP_OBJECTS_LV_DIF = 20,
};

class CBiologist : public singleton<CBiologist>
{
	public:
		CBiologist();
		~CBiologist();
	
	public:
		void BiologistOpenPacket(LPCHARACTER pkChar);
		void BiologistProvidesMaterialPacket(LPCHARACTER pkChar, BYTE byIsElixirUse, BYTE byIsBookTimeUse);
		void BiologistChosenAffectPacket(LPCHARACTER pkChar, BYTE byChosenAffect);
		void BiologistDecreaseTime(LPCHARACTER pkChar, BYTE byDecreaseTimeIndex);
		
		void SendBiologistClosePacket(LPCHARACTER pkChar);
		
		bool IsSelectiveAffect(LPCHARACTER pkChar);
		bool IsCompleted(LPCHARACTER pkChar);
		
		void SetTimeOut(LPCHARACTER pkChar, int iTimeOut);
		int GetTimeOut(LPCHARACTER pkChar) const;
		
		void DropObjects(LPCHARACTER pkChar, LPCHARACTER pkVictim, DWORD pkVictimVnum);
};
#endif
