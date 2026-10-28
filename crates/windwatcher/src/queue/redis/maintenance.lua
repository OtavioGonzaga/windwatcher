-- Phase 0 scans due leases in bounded pages; phase 1 prunes completed.
local phase = ARGV[1]
local minimum = ARGV[3] == '' and '-' or '(' .. ARGV[3]
local idx = phase == '0' and due('leased') or completed
local rows = redis.call('ZRANGEBYLEX',idx,minimum,'[' .. ARGV[2] .. ':~','LIMIT',0,64)
local changed = 0
for _,m in ipairs(rows) do
    local id = job_id(m)
    local k = hash(id)
    if phase == '0' then
        if redis.call('HGET',k,'status') ~= 'leased' then error('inconsistent queue index') end
        local attempts = tonumber(redis.call('HGET',k,'attempts'))
        local maximum = tonumber(redis.call('HGET',k,'max_attempts'))
        if not attempts or not maximum then error('invalid queue attempts') end
        if attempts >= maximum then exhausted(k,id,ARGV[2]); changed = changed+1 end
    else
        if redis.call('HGET',k,'status') ~= 'completed' then error('inconsistent queue index') end
        local listing = member(k,id)
        redis.call('ZREM',all,listing)
        redis.call('ZREM',index('completed'),listing)
        redis.call('ZREM',completed,m)
        redis.call('DEL',k)
        changed = changed+1
    end
end
return {tostring(changed), #rows == 64 and rows[#rows] or ''}
