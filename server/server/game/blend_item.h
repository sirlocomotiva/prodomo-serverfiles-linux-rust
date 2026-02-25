#pragma once
#include <map>
#include <memory>
#include <cstring>


class CBlendItem : public singleton<CBlendItem>
{
public:
	CBlendItem() = default;
	virtual ~CBlendItem();
	bool Initialize();
	bool FindItem(uint32_t item_vnum) { return blend_.find(item_vnum) != blend_.end(); }
	bool SetItemValue(LPITEM pkItem);
private:
	struct blend
	{
		static const auto item_max_value = 5;
		int32_t apply_type;
		int32_t apply_value[item_max_value];
		int32_t apply_duration[item_max_value];
		blend()
		{
			apply_type = 0;
			std::memset(&apply_value, 0, sizeof(apply_value));
			std::memset(&apply_duration, 0, sizeof(apply_duration));
		};
	};
	std::map<uint32_t, std::shared_ptr<blend>> blend_;
};
