#ifndef __INC_METIN_II_GAME_ITEM_H__
#define __INC_METIN_II_GAME_ITEM_H__

#include "entity.h"

class CItem : public CEntity
{
	protected:
		// override methods from ENTITY class
		virtual void	EncodeInsertPacket(LPENTITY entity);
		virtual void	EncodeRemovePacket(LPENTITY entity);
		
#ifdef OFFLINE_SHOP
	public:
		void		SetRealID(DWORD id)		{ m_dwRealID = id; }
		DWORD		GetRealID()			{ return m_dwRealID; }
	protected:
		DWORD			m_dwRealID;
#endif


	public:
		CItem(DWORD dwVnum);
		virtual ~CItem();

		int			GetLevelLimit();

		int			GetLevelChampionLimit();

		bool		CheckItemUseLevel(int nLevel);
		
		bool		CheckItemUseLevelChampion(int nLevel);

		bool		IsPCBangItem();

		long		FindApplyValue(BYTE bApplyType);

		bool		IsStackable() const { return (GetFlag() & ITEM_FLAG_STACKABLE)?true:false; }

		void		Initialize();
		void		Destroy();

		void		Save();

		void		SetWindow(BYTE b)	{ m_bWindow = b; }
		BYTE		GetWindow()		{ return m_bWindow; }
		
		int			GetWindowInventoryEx();
		
		void		SetID(DWORD id)		{ m_dwID = id;	}
		DWORD		GetID()			{ return m_dwID; }

		void			SetProto(const TItemTable * table);
		TItemTable const *	GetProto()	{ return m_pProto; }

#ifdef ENABLE_REMOVE_LIMIT_GOLD
		unsigned long long		GetGold();
		unsigned long long		GetShopBuyPrice();
#else
		int		GetGold();
		int		GetShopBuyPrice();
#endif
#ifdef __MULTI_LANGUAGE_SYSTEM__
	const char* GetName();
#else
	const char* GetName() { return m_pProto ? m_pProto->szLocaleName : NULL; }
#endif
		const char *	GetBaseName()		{ return m_pProto ? m_pProto->szName : NULL; }
		BYTE		GetSize()		{ return m_pProto ? m_pProto->bSize : 0;	}

		void		SetFlag(long flag)	{ m_lFlag = flag;	}
		long		GetFlag() const { return m_lFlag;	}

		void		AddFlag(long bit);
		void		RemoveFlag(long bit);

		DWORD		GetWearFlag()		{ return m_pProto ? m_pProto->dwWearFlags : 0; }
		DWORD		GetAntiFlag()		{ return m_pProto ? m_pProto->dwAntiFlags : 0; }
		DWORD		GetImmuneFlag()		{ return m_pProto ? m_pProto->dwImmuneFlag : 0; }


#ifdef ENABLE_CUSTOM_INVENTORY
		bool 		IsCustomCategory(BYTE bCategory) const;
		int 		GetItemCategory() const;
#endif

		void		SetVID(DWORD vid)	{ m_dwVID = vid;	}
		DWORD		GetVID()		{ return m_dwVID;	}

		bool		SetCount(DWORD count);
		DWORD		GetCount();
		inline 		void IncrementCount() { SetCount(GetCount() + 1); }
		inline 		bool DecrementCount() { return SetCount(GetCount() - 1); }
#ifdef ENABLE_REFINE_ELEMENT
		void		SetRefineElement(DWORD);
		DWORD		GetRefineElement() { return m_dwRefineElement; }
		BYTE 		GetRefineElementType() { return (!m_dwRefineElement) ? 0 : ((BYTE)(m_dwRefineElement / 100000000)); }
		BYTE 		GetRefineElementPlus() { return (!m_dwRefineElement) ? 0 : ((BYTE)(m_dwRefineElement / 10000000 % 10)); }
		BYTE 		GetRefineElementBonusValue() { return (!m_dwRefineElement) ? 0 : ((BYTE)(m_dwRefineElement / 100000 % 100)); }
		BYTE 		GetRefineElementAttackValue() { return (!m_dwRefineElement) ? 0 : ((BYTE)(m_dwRefineElement / 1000 % 100)); }
		BYTE 		GetRefineElementLastIncBonus() { return (!m_dwRefineElement) ? 0 : ((BYTE)(m_dwRefineElement / 100 % 10)); }
		BYTE 		GetRefineElementLastIncAttack() { return (!m_dwRefineElement) ? 0 : ((BYTE)(m_dwRefineElement % 100)); }
#endif


		DWORD		GetVnum() const		{ return m_dwMaskVnum ? m_dwMaskVnum : m_dwVnum;	}
		DWORD		GetOriginalVnum() const		{ return m_dwVnum;	}
		BYTE		GetType() const		{ return m_pProto ? m_pProto->bType : 0;	}
		BYTE		GetSubType() const	{ return m_pProto ? m_pProto->bSubType : 0;	}
		BYTE		GetLimitType(DWORD idx) const { return m_pProto ? m_pProto->aLimits[idx].bType : 0;	}
		long		GetLimitValue(DWORD idx) const { return m_pProto ? m_pProto->aLimits[idx].lValue : 0;	}

		long		GetValue(DWORD idx);

		void		SetCell(LPCHARACTER ch, WORD pos)	{ m_pOwner = ch, m_wCell = pos;	}
		WORD		GetCell()				{ return m_wCell;	}
		
		uint16_t GetApplyType(int i) { return m_pProto ? m_pProto->aApplies[i].bType : 0; } //@fixme532
		long GetApplyValue(int i) { return m_pProto ? m_pProto->aApplies[i].lValue : 0; }

		LPITEM		RemoveFromCharacter();
#if defined(__BL_ENABLE_PICKUP_ITEM_EFFECT__)
		bool	AddToCharacter(LPCHARACTER ch, TItemPos Cell, bool bHighlight = true);
#else
		bool	AddToCharacter(LPCHARACTER ch, TItemPos Cell);
#endif
		LPCHARACTER	GetOwner()		{ return m_pOwner; }

		LPITEM		RemoveFromGround();
		bool		AddToGround(long lMapIndex, const PIXEL_POSITION & pos, bool skipOwnerCheck = false);

		int			FindEquipCell(LPCHARACTER ch, int bCandidateCell = -1);
		bool		IsEquipped() const		{ return m_bEquipped;	}
		bool		EquipTo(LPCHARACTER ch, BYTE bWearCell);
		bool		IsEquipable() const;

		bool		CanUsedBy(LPCHARACTER ch);

		bool		DistanceValid(LPCHARACTER ch);

		void		UpdatePacket();
		void		UsePacketEncode(LPCHARACTER ch, LPCHARACTER victim, struct packet_item_use * packet);

		void		SetExchanging(bool isOn = true);
		bool		IsExchanging() const { return m_bExchanging;	}

		bool		IsTwohanded();

		bool		IsPolymorphItem();

		void		ModifyPoints(bool bAdd);	// �������� ȿ���� ĳ���Ϳ� �ο� �Ѵ�. bAdd�� false�̸� ������

		bool		CreateSocket(BYTE bSlot, BYTE bGold);
		const long *	GetSockets()		{ return &m_alSockets[0];	}
		long		GetSocket(int i) const { return m_alSockets[i];	}

		void		SetSockets(const long * al);
		void		SetSocket(int i, long v, bool bLog = true);

		int		GetSocketCount();
		bool		AddSocket();

		const TPlayerItemAttribute* GetAttributes()		{ return m_aAttr;	}
		const TPlayerItemAttribute& GetAttribute(int i)	{ return m_aAttr[i];	}

		BYTE		GetAttributeType(int i)	{ return m_aAttr[i].bType;	}
		short		GetAttributeValue(int i){ return m_aAttr[i].sValue;	}

		void		SetAttributes(const TPlayerItemAttribute* c_pAttribute);

		int		FindAttribute(BYTE bType);
		bool		RemoveAttributeAt(int index);
		bool		RemoveAttributeType(BYTE bType);

		bool		HasAttr(BYTE bApply);
		bool		HasRareAttr(BYTE bApply);

		void		SetDestroyEvent(LPEVENT pkEvent);
		void		StartDestroyEvent(int iSec=300);

		DWORD		GetRefinedVnum()	{ return m_pProto ? m_pProto->dwRefinedVnum : 0; }
		DWORD		GetRefineFromVnum();
		int		GetRefineLevel();
#if defined(__ATTR_6TH_7TH__)
	DWORD GetAttr67MaterialVnum();
#endif	
#ifdef __CHANGELOOK_SYSTEM__
		DWORD	GetTransmutation() const	{return m_dwTransmutation;}
		void	SetTransmutation(DWORD dwVnum, bool bLog = false);
#endif

		void		SetSkipSave(bool b)	{ m_bSkipSave = b; }
		bool		GetSkipSave()		{ return m_bSkipSave; }

		bool		IsOwnership(LPCHARACTER ch);
		void		SetOwnership(LPCHARACTER ch, int iSec = 10);
		void		SetOwnershipEvent(LPEVENT pkEvent);

		DWORD		GetLastOwnerPID()	{ return m_dwLastOwnerPID; }
		void		SetLastOwnerPID(DWORD pid) { m_dwLastOwnerPID = pid; }
		const 		TItemLimit* FindLimit(uint8_t type) const;
		int			GetAttributeSetIndex(); // �Ӽ� �ٴ°��� ������ �迭�� ��� �ε����� ����ϴ��� �����ش�.
		void		AlterToMagicItem();
		void		AlterToSocketItem(int iSocketCount);

		WORD		GetRefineSet()		{ return m_pProto ? m_pProto->wRefineSet : 0;	}

		void		StartUniqueExpireEvent();
		void		SetUniqueExpireEvent(LPEVENT pkEvent);

		void		StartTimerBasedOnWearExpireEvent();
		void		SetTimerBasedOnWearExpireEvent(LPEVENT pkEvent);

		void		StartRealTimeExpireEvent();
		bool		IsRealTimeItem();

#ifdef ENABLE_AFFECT_RENEWAL
		void		StartBlendExpireEvent();
		void		StopBlendExpireEvent();
#endif

		void		StopUniqueExpireEvent();
		void		StopTimerBasedOnWearExpireEvent();
		void		StopAccessorySocketExpireEvent();

		//			�ϴ� REAL_TIME�� TIMER_BASED_ON_WEAR �����ۿ� ���ؼ��� ����� ������.
		int			GetDuration();

		int		GetAttributeCount();
		void		ClearAttribute();
		void		ChangeAttribute(const int* aiChangeProb=NULL);
		void		AddAttribute();
		void		AddAttribute(BYTE bType, short sValue);

		void 		ApplyAddon(int iAddonType);


		int		GetSpecialGroup() const;
		bool	IsSameSpecialGroup(const LPITEM item) const;

		// ACCESSORY_REFINE
		// �׼������� ������ ���� ������ �߰�
		bool		IsAccessoryForSocket();

		int		GetAccessorySocketGrade();
		int		GetAccessorySocketMaxGrade();
		int		GetAccessorySocketDownGradeTime();

		void		SetAccessorySocketGrade(int iGrade);
		void		SetAccessorySocketMaxGrade(int iMaxGrade);
		void		SetAccessorySocketDownGradeTime(DWORD time);

		void		AccessorySocketDegrade();

		// �Ǽ��縮 �� �����ۿ� �۾����� Ÿ�̸� ���ư��°�( ����, �� )
		void		StartAccessorySocketExpireEvent();
		void		SetAccessorySocketExpireEvent(LPEVENT pkEvent);

		bool		CanPutInto(LPITEM item);
		// END_OF_ACCESSORY_REFINE

		void		CopyAttributeTo(LPITEM pItem);
		void		CopySocketTo(LPITEM pItem);

		int			GetRareAttrCount();
		bool		AddRareAttribute();
		bool		ChangeRareAttribute();
		
		const int CustomSort() const {
			switch (m_pProto->bType) {
				case ITEM_WEAPON:
					return 1;
				case ITEM_ARMOR:
					return 2;
				case ITEM_USE:
					return 3;
				case ITEM_BELT:
					return 4;
				case ITEM_COSTUME:
					return 5;
				case ITEM_SKILLBOOK:
				case ITEM_SKILLFORGET:
					return 6;
				case ITEM_METIN:
					return 7;
				case ITEM_MATERIAL:
					return 8;
			}
			return 9;
		}
		
		void		AttrLog();

		void		Lock(bool f) { m_isLocked = f; }
		bool		isLocked() const { return m_isLocked; }
		
		
#ifdef __PREMIUM_PRIVATE_SHOP__
		void				BindPrivateShop(LPPRIVATE_SHOP pPrivateShop) { m_pPrivateShop = pPrivateShop; }
		LPPRIVATE_SHOP		GetPrivateShop() { return m_pPrivateShop; }

		void		SetGoldPrice(long long llGoldPrice) { m_llGoldPrice = llGoldPrice; }
		long long	GetGoldPrice() { return m_llGoldPrice; }

		void		SetChequePrice(DWORD dwChequePrice) { m_dwChequePrice = dwChequePrice; }
		DWORD		GetChequePrice() { return m_dwChequePrice; }

		void		SetCheckinTime(time_t tCheckin) { m_tPrivateShopCheckin = tCheckin; }
		time_t		GetCheckinTime() { return m_tPrivateShopCheckin; }
#endif
		

	private :
		void		SetAttribute(int i, BYTE bType, short sValue);
	public:
		void		SetForceAttribute(int i, BYTE bType, short sValue);

	protected:
		bool		EquipEx(bool is_equip);
		bool		Unequip();

		void		AddAttr(BYTE bApply, BYTE bLevel);
		void		PutAttribute(const int * aiAttrPercentTable);
		void		PutAttributeWithLevel(BYTE bLevel);

	public:
		void		AddRareAttribute2(const int * aiAttrPercentTable = NULL);
	protected:
		void		AddRareAttr(BYTE bApply, BYTE bLevel);
		void		PutRareAttribute(const int * aiAttrPercentTable);
		void		PutRareAttributeWithLevel(BYTE bLevel);

	protected:
		friend class CInputDB;
#ifdef __PREMIUM_PRIVATE_SHOP__
		friend class CPrivateShop;
		friend class CPrivateShopManager;
#endif
		bool		OnAfterCreatedItem();			// ������ �������� ��� ������ �Բ� ������ ����(�ε�)�� �� �Ҹ���� �Լ�.

	public:
		bool		IsRideItem();
		bool		IsRamadanRing();

		void		ClearMountAttributeAndAffect();
		bool		IsNewMountItem();
		bool 		IsTalisman() const { return GetType() == ITEM_TALISMAN;}
#ifdef ENABLE_GLOVE_SYSTEM
		bool 		IsGloves() { return GetType() == ITEM_ARMOR && GetSubType() == ARMOR_GLOVE; }
#endif
#ifdef ENABLE_MOUNT_COSTUME_SYSTEM
		bool		IsMountItem();
		bool		IsPermanentMount();
#endif

#ifdef ENABLE_PET_COSTUME_SYSTEM
	bool			IsPetItem();
#endif
		void		SetMaskVnum(DWORD vnum)	{	m_dwMaskVnum = vnum; }
		DWORD		GetMaskVnum()			{	return m_dwMaskVnum; }
		bool		IsMaskedItem()	{	return m_dwMaskVnum != 0;	}
#ifdef ENABLE_AFFECT_RENEWAL
		bool 		IsBlendItem() { return GetType() == ITEM_BLEND; }
#endif
		// ��ȥ��
		bool		IsDragonSoul();
		bool		IsSash();
		bool		IsSashSkin();
		int		GiveMoreTime_Per(float fPercent);
		int		GiveMoreTime_Fix(DWORD dwTime);


	private:
		TItemTable const * m_pProto;		// ������ Ÿ��

		DWORD		m_dwVnum;
		LPCHARACTER	m_pOwner;

		BYTE		m_bWindow;		// ���� �������� ��ġ�� ������
		DWORD		m_dwID;			// ������ȣ
		bool		m_bEquipped;	// ���� �Ǿ��°�?
		DWORD		m_dwVID;		// VID
		WORD		m_wCell;		// ��ġ
		DWORD		m_dwCount;		// ����
#ifdef ENABLE_REFINE_ELEMENT
		DWORD		m_dwRefineElement;
#endif
#ifdef __CHANGELOOK_SYSTEM__
		DWORD		m_dwTransmutation;
#endif
		long		m_lFlag;		// �߰� flag
		DWORD		m_dwLastOwnerPID;	// ������ ������ �־��� ����� PID

		bool		m_bExchanging;	///< ���� ��ȯ�� ����

		long		m_alSockets[ITEM_SOCKET_MAX_NUM];	// ������ ��Ĺ
		TPlayerItemAttribute	m_aAttr[ITEM_ATTRIBUTE_MAX_NUM];
		LPEVENT		m_pkDestroyEvent;
		LPEVENT		m_pkExpireEvent;
		LPEVENT		m_pkUniqueExpireEvent;
		LPEVENT		m_pkTimerBasedOnWearExpireEvent;
		LPEVENT		m_pkRealTimeExpireEvent;
		LPEVENT		m_pkAccessorySocketExpireEvent;
		LPEVENT		m_pkOwnershipEvent;
#ifdef ENABLE_AFFECT_RENEWAL
		LPEVENT 	m_pkBlendUseEvent;
#endif
		DWORD		m_dwOwnershipPID;

		bool		m_bSkipSave;

		bool		m_isLocked;

		DWORD		m_dwMaskVnum;
		DWORD		m_dwSIGVnum;
		
#ifdef __PREMIUM_PRIVATE_SHOP__
		LPPRIVATE_SHOP		m_pPrivateShop;
		long long			m_llGoldPrice;
		DWORD				m_dwChequePrice;
		time_t				m_tPrivateShopCheckin;
#endif
		
	public:
		void SetSIGVnum(DWORD dwSIG)
		{
			m_dwSIGVnum = dwSIG;
		}
		DWORD	GetSIGVnum() const
		{
			return m_dwSIGVnum;
		}
#ifdef ENABLE_SEND_TARGET_INFO_EXTENDED
	public:
		void		SetRarity(DWORD rarity) { dwRarity = rarity; }
		DWORD		GetRarity() { return dwRarity; }
	protected:
		DWORD		dwRarity;
#endif
#ifdef __AURA_SYSTEM__
	private:
		LPEVENT m_pkAuraBoostSocketExpireEvent;

	public:
		bool	IsAuraBoosterForSocket();

		void	StartAuraBoosterSocketExpireEvent();
		void	StopAuraBoosterSocketExpireEvent();
		void	SetAuraBoosterSocketExpireEvent(LPEVENT pkEvent);
#endif
	public:
		bool IsRarityItem();
};

EVENTINFO(item_event_info)
{
	LPITEM item;
	char szOwnerName[CHARACTER_NAME_MAX_LEN];

	item_event_info()
	: item( 0 )
	{
		::memset( szOwnerName, 0, CHARACTER_NAME_MAX_LEN );
	}
};

EVENTINFO(item_vid_event_info)
{
	DWORD item_vid;

	item_vid_event_info()
	: item_vid( 0 )
	{
	}
};

#endif
