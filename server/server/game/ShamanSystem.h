#ifndef	__HEADER_SHAMAN_ACTOR_SYSTEM__
#define	__HEADER_SHAMAN_ACTOR_SYSTEM__

#include <unordered_map>

class CHARACTER;
class CItem;

class CShamanActor //: public CHARACTER
{
	friend class CHARACTER;

public:

	CShamanActor(CHARACTER& owner);
	virtual ~CShamanActor();

	CHARACTER& GetOwner() const { return m_owner; }
	uint32_t GetVnum() const { return m_dwVnum; }
	CHARACTER* GetCharacter() const { return m_pkChar; }

	uint32_t Summon(std::string shamanName, bool bSpawnFar = false);
	void Unsummon();
	
	void UseSkills();
	void UseSkill(BYTE dwVnum);
	int GetBuffValue(BYTE dwVnum) const;
	BYTE GetBuffLevel(BYTE dwVnum) const;
	void SetBuffLevel(BYTE dwVnum, BYTE byLevel);
	void UpgradeSkill(uint32_t dwVnum, LPITEM pkItem);
	bool TrainByQuest(BYTE dwVnum);
	bool IsTrainingSkill(uint32_t dwVnum) const;
	
	bool HasPremium() const;
	bool SetPremium(LPITEM pkItem);
	
	bool IsSummoned() const { return m_pkChar; }
	
	void UpdateInformations();
	void UpdateEquipment();
	
	virtual bool Update();

protected:
	void OnDestroy() { m_pkChar = nullptr; }
	
	virtual bool UpdateFollowAI(); ///< ??? ????? AI ??
	virtual bool UpdatAloneActionAI(float fMinDist, float fMaxDist); ///< ?? ???? ?? ?? AI ??

	bool Follow(int32_t minDistance = 50) const;

	CHARACTER& m_owner;
	uint32_t m_dwVnum;
	uint32_t m_dwLastActionTime;
	uint32_t m_dwLastSkillTime;
	
	CHARACTER* m_pkChar; // Instance of shaman(CHARACTER)
};

class CShamanSystem
{
	friend EVENTFUNC(shamansystem_update_event);

public:
	CShamanSystem(CHARACTER& owner);
	~CShamanSystem();

	bool Update();

	CShamanActor* SummonItem(uint32_t itemId, bool bSpawnFar);

	void UnsummonItem(uint32_t itemId);
	bool TrainSkill(uint32_t dwSkillVnum, LPITEM pkItem);
	bool TrainByQuest(BYTE bySkillVnum);
	BYTE GetSkillLevel(BYTE bySkillVnum);
	
	bool UpgradePremium(LPITEM pkItem);
	void SendInformations();
	
private:
	CHARACTER& m_owner;
	std::unordered_map<uint32_t, std::unique_ptr<CShamanActor>> m_ShamanItemsMap;
	
	// Pet update
	uint32_t m_dwUpdatePeriod;
	uint32_t m_dwLastUpdateTime;
	LPEVENT m_pkShamanUpdateEvent;
};

#endif